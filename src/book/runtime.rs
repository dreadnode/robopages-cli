use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use lazy_regex::{lazy_regex, Lazy};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::runtime::CommandLine;

use super::{Function, Page};

static ARG_VALUE_PARSER: Lazy<Regex> = lazy_regex!(r"(?m)\$\{\s*([\w\.]+)(\s+or\s+([^}]+))?\}");

const ARG_EXPRESSION_ERROR: &str =
    "argument expression must be in the form of ${name} or ${name or default_value}";

#[allow(dead_code)]
pub enum ExecutionFlavor {
    Shell(String),
    Sudo,
    Docker(String),
    Error(String),
}

impl ExecutionFlavor {
    pub fn shell(shell: String) -> Self {
        ExecutionFlavor::Shell(shell)
    }

    pub fn sudo() -> Self {
        ExecutionFlavor::Sudo
    }

    pub fn docker(image: String) -> Self {
        ExecutionFlavor::Docker(image)
    }

    pub fn error(message: String) -> Self {
        ExecutionFlavor::Error(message)
    }

    fn get_current_shell() -> String {
        let shell_name = std::env::var("SHELL")
            .map(|s| s.split('/').last().unwrap_or("unknown").to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        if let Ok(shell_path) = which::which(shell_name.clone()) {
            shell_path.to_string_lossy().to_string()
        } else {
            shell_name
        }
    }

    pub fn for_function(function: &Function) -> anyhow::Result<ExecutionFlavor> {
        let mut has_container = false;
        if let Some(container) = function.container.as_ref() {
            has_container = true;
            if container.force {
                return Ok(ExecutionFlavor::docker(
                    container.source.image().to_string(),
                ));
            }
        }

        match function.execution.get_command_line() {
            Ok(raw_parts) => {
                let cmdline = CommandLine::from_vec(&raw_parts)?;
                if cmdline.sudo {
                    return Ok(if has_container {
                        ExecutionFlavor::docker(
                            function
                                .container
                                .as_ref()
                                .unwrap()
                                .source
                                .image()
                                .to_string(),
                        )
                    } else {
                        ExecutionFlavor::sudo()
                    });
                } else if !cmdline.app_in_path {
                    return Ok(if has_container {
                        ExecutionFlavor::docker(
                            function
                                .container
                                .as_ref()
                                .unwrap()
                                .source
                                .image()
                                .to_string(),
                        )
                    } else {
                        ExecutionFlavor::error("app not in $PATH".to_string())
                    });
                } else {
                    return Ok(ExecutionFlavor::shell(Self::get_current_shell()));
                }
            }
            Err(e) => Err(e),
        }
    }
}

impl std::fmt::Display for ExecutionFlavor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Shell(shell) => shell.to_string(),
            Self::Sudo => "sudo".to_string(),
            Self::Docker(image) => format!("docker {}", image),
            Self::Error(message) => message.to_string(),
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ExecutionContext {
    #[serde(rename = "cmdline")]
    CommandLine(Vec<String>),
    #[serde(rename = "platforms")]
    PlatformSpecific(BTreeMap<String, Vec<String>>),
}

impl ExecutionContext {
    pub fn get_command_line(&self) -> anyhow::Result<Vec<String>> {
        match self {
            Self::CommandLine(cmdline) => Ok(cmdline.clone()),
            Self::PlatformSpecific(platforms) => {
                if let Some(cmdline) = platforms.get(std::env::consts::OS) {
                    Ok(cmdline.clone())
                } else {
                    Err(anyhow::anyhow!(
                        "no command line for platform {}",
                        std::env::consts::OS
                    ))
                }
            }
        }
    }
}

#[allow(dead_code)] // we might need path and page in the future
#[derive(Debug)]
pub struct FunctionRef<'a> {
    pub name: String,
    pub path: &'a Utf8PathBuf,
    pub page: &'a Page,
    pub function: &'a Function,
}

impl<'a> FunctionRef<'a> {
    pub fn validate_arguments(
        &self,
        provided_arguments: &BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        // check for missing required arguments
        for (arg_name, param) in &self.function.parameters {
            if param.required && !provided_arguments.contains_key(arg_name) {
                return Err(anyhow::anyhow!(
                    "missing required argument {} for function {}",
                    arg_name,
                    &self.name
                ));
            }
        }

        // check for extra arguments
        for arg_name in provided_arguments.keys() {
            if !self.function.parameters.contains_key(arg_name) {
                return Err(anyhow::anyhow!(
                    "unknown argument {} for function {}",
                    arg_name,
                    &self.name
                ));
            }
        }

        Ok(())
    }

    pub fn resolve_command_line(
        &self,
        arguments: &BTreeMap<String, String>,
    ) -> anyhow::Result<CommandLine> {
        // determine the command line to execute
        let command_line = self.function.execution.get_command_line()?;
        let mut env = BTreeMap::new();

        // interpolate the arguments
        let command_line = {
            let mut interpolated = Vec::new();
            for arg in command_line {
                interpolated.push(if let Some(caps) = ARG_VALUE_PARSER.captures(&arg) {
                    let var_name = caps
                        .get(1)
                        .ok_or(ARG_EXPRESSION_ERROR)
                        .map_err(|e| anyhow!(e))?
                        .as_str();
                    let var_default = caps.get(3).map(|m| m.as_str());
                    let full_match = caps.get(0).unwrap().as_str();

                    // read variable from the environment if the name starts with env. or ENV.
                    let replacement_value = if var_name.starts_with("env.") || var_name.starts_with("ENV.") {
                        let env_var_name = var_name.replace("env.", "").replace("ENV.", "");
                        let env_var = std::env::var(&env_var_name);
                        let env_var_value = if let Ok(value) = env_var {
                            value
                        } else if let Some(def) = var_default {
                            def.to_string()
                        } else {
                            return Err(anyhow::anyhow!(
                                "environment variable {} not set",
                                env_var_name
                            ));
                        };

                        // add the environment variable to the command line for later use
                        env.insert(env_var_name, env_var_value.to_owned());

                        env_var_value
                    } else if let Some(value) = arguments.get(var_name) {
                        // if the value is empty and there's a default value, use the default value
                        if value.is_empty() {
                            if let Some(def) = var_default {
                                def.to_string()
                            } else {
                                value.to_string()
                            }
                        } else {
                            // otherwise, use the provided value
                            value.to_string()
                        }
                    } else if let Some(default_value) = var_default {
                        // if the value is not provided and there's a default value, use the default value
                        default_value.to_string()
                    } else {
                        return Err(anyhow::anyhow!("argument {} not provided", var_name));
                    };
                    
                    // Replace only the pattern within the argument
                    arg.replace(full_match, &replacement_value)
                } else {
                    arg
                });
            }
            interpolated
        };

        // final parsing
        CommandLine::from_vec_with_env(&command_line, env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_resolve_command_line_with_valid_arguments() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${message}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let mut arguments = BTreeMap::new();
        arguments.insert("message".to_string(), "Hello, World!".to_string());

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["Hello, World!"]);
    }

    #[test]
    fn test_resolve_command_line_with_default_value() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${message or Default message}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let arguments = BTreeMap::new();

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["Default message"]);
    }

    #[test]
    fn test_resolve_command_line_with_empty_value_and_default() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${message or Default message}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let mut arguments = BTreeMap::new();
        arguments.insert("message".to_string(), "".to_string());

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["Default message"]);
    }

    #[test]
    fn test_resolve_command_line_with_missing_required_argument() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${required_arg}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let arguments = BTreeMap::new();

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "argument required_arg not provided"
        );
    }

    #[test]
    fn test_resolve_command_line_with_multiple_arguments() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${arg1}".to_string(),
                "${arg2 or default}".to_string(),
                "literal".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let mut arguments = BTreeMap::new();
        arguments.insert("arg1".to_string(), "value1".to_string());

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["value1", "default", "literal"]);
    }

    #[test]
    fn test_resolve_command_line_with_env_variables() {
        std::env::set_var("TEST_VAR", "test_value");

        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${env.TEST_VAR}".to_string(),
                "${ENV.TEST_VAR}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let arguments = BTreeMap::new();

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["test_value", "test_value"]);

        std::env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_resolve_command_line_with_undefined_env_variable() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${env.UNDEFINED_VAR}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let arguments = BTreeMap::new();

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "environment variable UNDEFINED_VAR not set"
        );
    }

    #[test]
    fn test_resolve_command_line_with_undefined_env_variable_with_default() {
        let function = Function {
            execution: ExecutionContext::CommandLine(vec![
                "echo".to_string(),
                "${env.UNDEFINED_VAR or default_value}".to_string(),
            ]),
            description: "".to_string(),
            parameters: BTreeMap::new(),
            container: None,
        };
        let resolver = FunctionRef {
            function: &function,
            name: "test_function".to_string(),
            path: &Utf8PathBuf::from("test/path"),
            page: &Page {
                name: "test_page".to_string(),
                description: None,
                categories: Vec::new(),
                functions: BTreeMap::new(),
            },
        };
        let arguments = BTreeMap::new();

        let result = resolver.resolve_command_line(&arguments);
        assert!(result.is_ok());
        let command_line = result.unwrap();
        assert!(command_line.app.ends_with("/echo"));
        assert_eq!(command_line.args, vec!["default_value"]);
    }
}
