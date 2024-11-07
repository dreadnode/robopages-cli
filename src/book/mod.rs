use std::collections::{BTreeMap, HashMap};

use camino::Utf8PathBuf;
use glob::glob;
use serde::{Deserialize, Serialize};

use crate::runtime::{CommandLine, ContainerSource};

pub(crate) mod flavors;
pub(crate) mod runtime;
pub(crate) mod templates;

macro_rules! eval_if_in_filter {
    ($path:expr, $filter:expr, $action:expr) => {
        // include by default
        let mut include = true;
        // if filter is set
        if let Some(filter) = &$filter {
            // if it does not match, do not include
            if !$path.as_str().contains(filter) {
                include = false;
            }
        }
        if include {
            $action
        }
    };
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Parameter {
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    #[serde(default = "default_required")]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
}

fn default_required() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Container {
    #[serde(flatten)]
    pub source: ContainerSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<String>>,
    #[serde(default = "default_force")]
    #[serde(skip_serializing_if = "is_false")]
    pub force: bool,
    #[serde(default = "default_preserve_app")]
    #[serde(skip_serializing_if = "is_false")]
    pub preserve_app: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !(*b)
}

fn default_force() -> bool {
    false
}

fn default_preserve_app() -> bool {
    false
}

impl Container {
    pub fn wrap(&self, cmdline: CommandLine) -> anyhow::Result<CommandLine> {
        let mut dockerized = CommandLine {
            sudo: false,
            app: which::which("docker")
                .map_err(|e| anyhow::anyhow!("docker executable not found: {}", e))?
                .to_string_lossy()
                .to_string(),
            app_in_path: true,
            args: vec!["run".to_string(), "--rm".to_string()],
            env: BTreeMap::new(),
            temp_env_file: None,
        };

        // handle environment variables if present
        if !cmdline.env.is_empty() {
            let mut env_contents = String::new();
            for (key, value) in &cmdline.env {
                env_contents.push_str(&format!("{}={}\n", key, value));
            }

            // create temp file
            let temp_file = tempfile::NamedTempFile::new()
                .map_err(|e| anyhow::anyhow!("failed to create temp env file: {}", e))?;

            // write env vars
            std::fs::write(temp_file.path(), env_contents)
                .map_err(|e| anyhow::anyhow!("failed to write env file: {}", e))?;

            // add env-file arg
            dockerized
                .args
                .push(format!("--env-file={}", temp_file.path().display()));

            // keep temp file alive until docker run completes
            dockerized.temp_env_file = Some(temp_file);
        }

        // add volumes if any
        if let Some(volumes) = &self.volumes {
            for volume in volumes {
                dockerized.args.push(format!("-v{}", volume));
                dockerized.args.push(volume.to_string());
            }
        }

        // add any additional args
        if let Some(args) = &self.args {
            dockerized.args.extend(args.clone());
        }

        // add image
        dockerized.args.push(self.source.image().to_string());

        if self.preserve_app {
            // add the original app to the args
            dockerized.args.push(cmdline.app.clone());
        }

        // add the original arguments
        dockerized.args.extend(cmdline.args);

        Ok(dockerized)
    }

    pub async fn resolve(&self) -> anyhow::Result<()> {
        self.source.resolve(self.platform.clone()).await
    }
}

// TODO: add optional parsers to reduce output tokens

#[derive(Debug, Serialize, Deserialize)]
pub struct Function {
    pub description: String,
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<Container>,
    #[serde(flatten)]
    pub execution: runtime::ExecutionContext,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(default = "String::new")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub functions: BTreeMap<String, Function>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default = "Vec::new")]
    pub categories: Vec<String>,
}

impl Page {
    fn preprocess(path: &Utf8PathBuf, text: String) -> anyhow::Result<String> {
        let path = path.canonicalize_utf8()?;
        let base_path = path.parent().unwrap();

        Ok(text.replace("${cwd}", base_path.as_ref()))
    }

    pub fn from_path(path: &Utf8PathBuf) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("error while reading {:?}: {}", path, e))?;
        let text = Self::preprocess(path, text)
            .map_err(|e| anyhow::anyhow!("error while preprocessing {:?}: {}", path, e))?;
        let page = serde_yaml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("error while parsing {:?}: {}", path, e))?;
        Ok(page)
    }
}

#[derive(Debug)]
pub struct Book {
    pub pages: BTreeMap<Utf8PathBuf, Page>,
}

impl Book {
    pub fn from_path(path: Utf8PathBuf, filter: Option<String>) -> anyhow::Result<Self> {
        log::info!("Searching for pages in {:?}", path);
        let mut page_paths = Vec::new();

        let path = Utf8PathBuf::from(
            shellexpand::full(path.as_str())
                .map_err(|e| anyhow::anyhow!("failed to expand path: {}", e))?
                .into_owned(),
        )
        .canonicalize_utf8()
        .map_err(|e| anyhow::anyhow!("failed to canonicalize path: {}", e))?;

        log::info!("Canonicalized path: {:?}", path);

        if path.is_file() {
            log::info!("Path is a file");
            eval_if_in_filter!(path, filter, page_paths.push(path.to_path_buf()));
        } else if path.is_dir() {
            log::info!("Path is a directory, searching for .yml files");
            let glob_pattern = path.join("**/*.yml").as_str().to_string();
            log::info!("Using glob pattern: {}", glob_pattern);

            for entry in glob(&glob_pattern)? {
                match entry {
                    Ok(entry_path) => {
                        log::debug!("Found file: {:?}", entry_path);
                        // skip files in hidden directories (starting with .)
                        // but allow the root .robopages directory
                        if let Ok(relative_path) = entry_path.strip_prefix(&path) {
                            if relative_path.components().any(|component| {
                                let comp_str = component.as_os_str().to_string_lossy();
                                comp_str.starts_with(".") && comp_str != "." && comp_str != ".."
                            }) {
                                log::debug!("Skipping hidden file/directory");
                                continue;
                            }
                        }

                        if let Ok(utf8_path) = Utf8PathBuf::from_path_buf(entry_path) {
                            eval_if_in_filter!(utf8_path, filter, {
                                log::debug!("Adding path: {:?}", utf8_path);
                                page_paths.push(utf8_path);
                            });
                        } else {
                            log::error!("failed to convert path to Utf8PathBuf");
                        }
                    }
                    Err(e) => {
                        log::error!("Error in glob: {}", e);
                    }
                }
            }
        }

        log::info!("Found {} page paths", page_paths.len());

        if page_paths.is_empty() {
            return Err(anyhow::anyhow!("no pages found in {:?}", path));
        }

        log::debug!("loading {} pages from {:?}", page_paths.len(), path);

        let mut pages = BTreeMap::new();
        let mut function_names = HashMap::new();

        for page_path in page_paths {
            let page_path = page_path.canonicalize_utf8()?;
            let mut page = Page::from_path(&page_path)?;

            // if name is not set, use the file name
            if page.name.is_empty() {
                page.name = page_path.file_stem().unwrap().to_string();
            }

            // if categories are not set, use the path components
            if page.categories.is_empty() {
                let path_buf = page_path.strip_prefix(&path)?;
                let parent = path_buf.parent();

                if let Some(parent_path) = parent {
                    page.categories = parent_path
                        .components()
                        .map(|c| c.as_str().to_string())
                        .collect();

                    // Skip empty categories
                    page.categories.retain(|c| !c.is_empty());
                }
            }

            // make sure function names are unique
            let mut renames = HashMap::new();
            for func_name in page.functions.keys() {
                if function_names.contains_key(func_name) {
                    let new_func_name = format!("{}_{}", &page.name, func_name);
                    if !function_names.contains_key(&new_func_name) {
                        log::warn!(
                            "function name {} in {:?} is not unique, renaming to {}",
                            func_name,
                            page_path,
                            new_func_name
                        );
                        renames.insert(func_name.clone(), new_func_name.clone());
                    } else {
                        return Err(anyhow::anyhow!(
                            "function name {} in {:?} is not unique",
                            func_name,
                            page_path
                        ));
                    }
                }
                function_names.insert(func_name.clone(), 1);
            }

            for (old_name, new_name) in renames {
                let function = page.functions.remove(&old_name).unwrap();
                page.functions.insert(new_name, function);
            }

            pages.insert(page_path, page);
        }

        Ok(Self { pages })
    }

    pub fn size(&self) -> usize {
        self.pages.len()
    }

    pub fn get_function<'a>(&'a self, name: &str) -> anyhow::Result<runtime::FunctionRef<'a>> {
        for (page_path, page) in &self.pages {
            if let Some(function) = page.functions.get(name) {
                return Ok(runtime::FunctionRef {
                    name: name.to_owned(),
                    path: page_path,
                    page,
                    function,
                });
            }
        }

        Err(anyhow::anyhow!("function {} not found", name))
    }

    pub fn as_tools<'a, T>(&'a self, filter: Option<String>) -> Vec<T>
    where
        Vec<T>: std::convert::From<&'a Page>,
    {
        let mut tools = Vec::new();

        for (page_path, page) in &self.pages {
            eval_if_in_filter!(
                page_path,
                filter,
                tools.extend(<&Page as Into<Vec<T>>>::into(page))
            );
        }

        tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use flavors::openai;
    use std::collections::BTreeMap;

    fn create_test_book() -> Book {
        let mut pages = BTreeMap::new();
        let mut page = Page {
            name: "Test Page".to_string(),
            description: Some("A test page".to_string()),
            categories: vec!["test".to_string()],
            functions: BTreeMap::new(),
        };
        page.functions.insert(
            "test_function".to_string(),
            Function {
                description: "A test function".to_string(),
                parameters: BTreeMap::new(),
                execution: runtime::ExecutionContext::CommandLine(vec![
                    "echo".to_string(),
                    "test".to_string(),
                ]),
                container: None,
            },
        );
        pages.insert(Utf8PathBuf::from("test_page"), page);
        Book { pages }
    }

    #[test]
    fn test_book_size() {
        let book = create_test_book();
        assert_eq!(book.size(), 1);
    }

    #[test]
    fn test_get_existing_function() {
        let book = create_test_book();
        let result = book.get_function("test_function");
        assert!(result.is_ok());
        let function_ref = result.unwrap();
        assert_eq!(function_ref.name, "test_function");
        assert_eq!(function_ref.path, &Utf8PathBuf::from("test_page"));
    }

    #[test]
    fn test_get_non_existing_function() {
        let book = create_test_book();
        let result = book.get_function("non_existing_function");
        assert!(result.is_err());
    }

    #[test]
    fn test_as_tools_without_filter() {
        let book = create_test_book();
        let tools = book.as_tools::<openai::Tool>(None);
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_as_tools_with_matching_filter() {
        let book = create_test_book();
        let tools = book.as_tools::<openai::Tool>(Some("test_page".to_string()));
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_as_tools_with_non_matching_filter() {
        let book = create_test_book();
        let tools = book.as_tools::<openai::Tool>(Some("non_existing_page".to_string()));
        assert_eq!(tools.len(), 0);
    }

    #[test]
    fn test_container_preserve_app() {
        let container = Container {
            source: ContainerSource::Image("test_image".to_string()),
            args: None,
            volumes: None,
            force: false,
            preserve_app: true,
            platform: None,
        };

        let original_cmdline = CommandLine {
            sudo: false,
            app: "original_app".to_string(),
            app_in_path: true,
            args: vec!["arg1".to_string(), "arg2".to_string()],
            env: BTreeMap::new(),
            temp_env_file: None,
        };

        let wrapped_cmdline = container.wrap(original_cmdline).unwrap();

        assert!(wrapped_cmdline.args.contains(&"original_app".to_string()));
        assert!(wrapped_cmdline.args.contains(&"arg1".to_string()));
        assert!(wrapped_cmdline.args.contains(&"arg2".to_string()));

        // check that the original app is inserted before its arguments
        let app_index = wrapped_cmdline
            .args
            .iter()
            .position(|arg| arg == "original_app")
            .unwrap();
        let arg1_index = wrapped_cmdline
            .args
            .iter()
            .position(|arg| arg == "arg1")
            .unwrap();
        let arg2_index = wrapped_cmdline
            .args
            .iter()
            .position(|arg| arg == "arg2")
            .unwrap();
        assert!(app_index < arg1_index);
        assert!(app_index < arg2_index);
    }

    #[test]
    fn test_book_creation_with_duplicate_function_names() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::with_prefix("robopage-test-").unwrap();
        let base_path = temp_dir.path();

        fs::write(
            base_path.join("page1.yml"),
            r#"
description: First page
categories: [test]
functions:
  duplicate_function:
    description: A function
    parameters: {}
    cmdline: [echo, test]
"#,
        )
        .unwrap();

        fs::write(
            base_path.join("page2.yml"),
            r#"
description: Second page
categories: [test]
functions:
  duplicate_function:
    description: Another function
    parameters: {}
    cmdline: [echo, test]
"#,
        )
        .unwrap();

        let result = Book::from_path(Utf8PathBuf::from(base_path.to_str().unwrap()), None).unwrap();

        assert_eq!(result.size(), 2);
        assert!(result.get_function("duplicate_function").is_ok());
        assert!(result.get_function("page2_duplicate_function").is_ok());
    }

    #[test]
    fn test_book_skips_hidden_directories() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::with_prefix("robopage-test-").unwrap();
        let base_path = temp_dir.path();

        // Create a visible directory with a valid page
        fs::create_dir(base_path.join("visible")).unwrap();
        fs::write(
            base_path.join("visible/page1.yml"),
            r#"
description: Visible page
categories: [test]
functions:
  function1:
    description: A function
    parameters: {}
    cmdline: [echo, test]
"#,
        )
        .unwrap();

        // Create a hidden directory with a page that should be skipped
        fs::create_dir(base_path.join(".hidden")).unwrap();
        fs::write(
            base_path.join(".hidden/page2.yml"),
            r#"
description: Hidden page
categories: [test]
functions:
  function2:
    description: Another function
    parameters: {}
    cmdline: [echo, test]
"#,
        )
        .unwrap();

        let result = Book::from_path(Utf8PathBuf::from(base_path.to_str().unwrap()), None).unwrap();

        // Should only find the page from the visible directory
        assert_eq!(result.size(), 1);
        assert!(result.get_function("function1").is_ok());
        assert!(result.get_function("function2").is_err());
    }

    #[test]
    fn test_wrap_with_env() {
        let env: BTreeMap<String, String> = {
            let mut env = BTreeMap::new();
            env.insert("TEST_VAR".to_string(), "test_value".to_string());
            env
        };

        let command_line =
            CommandLine::from_vec_with_env(&vec!["echo".to_string(), "test".to_string()], env)
                .unwrap();

        let container = Container {
            source: ContainerSource::Image("test_image".to_string()),
            args: None,
            volumes: None,
            force: false,
            preserve_app: true,
            platform: None,
        };

        let wrapped = container.wrap(command_line).unwrap();

        // Find the env-file argument
        let env_file_arg = wrapped
            .args
            .iter()
            .find(|arg| arg.starts_with("--env-file="))
            .expect("--env-file argument not found")
            .clone();

        // Extract the file path
        let env_file_path = env_file_arg
            .strip_prefix("--env-file=")
            .expect("Failed to strip --env-file= prefix");

        let env_file = std::path::Path::new(env_file_path);
        assert!(env_file.exists());

        // Read the env file contents
        let env_file_contents = std::fs::read_to_string(env_file).expect("Failed to read env file");

        // Verify it contains the expected environment variable
        assert!(env_file_contents.contains("TEST_VAR=test_value"));

        // Clean up the env file
        drop(wrapped);
        assert!(!env_file.exists(), "env file was not deleted");
    }
}
