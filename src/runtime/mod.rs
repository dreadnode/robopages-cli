use std::sync::{atomic::AtomicUsize, Arc};

use crate::book::{flavors::openai, Book};

mod cmd;
mod docker;

pub(crate) mod prompt;
pub(crate) mod ssh;

pub(crate) use cmd::CommandLine;
pub(crate) use docker::ContainerSource;
use ssh::SSHConnection;

static ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);

// https://stackoverflow.com/questions/29963449/golang-like-defer-in-rust
struct ScopeCall<F: FnOnce()> {
    c: Option<F>,
}
impl<F: FnOnce()> Drop for ScopeCall<F> {
    fn drop(&mut self) {
        self.c.take().unwrap()()
    }
}

macro_rules! expr {
    ($e: expr) => {
        $e
    };
} // tt hack
macro_rules! defer {
    ($($data: tt)*) => (
        let _scope_call = ScopeCall {
            c: Some(|| -> () { expr!({ $($data)* }) })
        };
    )
}

async fn wait_for_available_tasks(max_running_tasks: usize) {
    let wait = std::time::Duration::from_secs(1);
    loop {
        let active_tasks = ACTIVE_TASKS.load(std::sync::atomic::Ordering::Relaxed);
        if active_tasks < max_running_tasks {
            break;
        }

        log::debug!("waiting for our turn, {} active tasks", active_tasks);
        tokio::time::sleep(wait).await;
    }
}

pub(crate) async fn execute_call(
    ssh: Option<SSHConnection>,
    interactive: bool,
    max_running_tasks: usize,
    book: Arc<Book>,
    call: openai::Call,
) -> anyhow::Result<openai::CallResultMessage> {
    wait_for_available_tasks(max_running_tasks).await;

    // increment the active tasks counter
    ACTIVE_TASKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    defer! {
        log::debug!("decrementing active tasks counter");
        ACTIVE_TASKS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    log::debug!("processing call: {:?}", call);

    let function = book.get_function(&call.function.name)?;

    log::debug!("{} resolved to: {:?}", &call.function.name, &function);

    // fail early if the arguments are invalid
    function.validate_arguments(&call.function.arguments)?;

    log::debug!("arguments validated");

    let command_line = function.resolve_command_line(&call.function.arguments)?;

    log::debug!("command line: {:?}", command_line);

    // validate runtime requirements
    let container = function.function.container.as_ref();
    let mut needs_container = false;
    let mut can_ssh = false;

    // if --ssh was provided
    if let Some(ssh) = ssh.as_ref() {
        // check if the app is in $PATH on the ssh host
        can_ssh = ssh.app_in_path(&command_line.app).await?;
        if !can_ssh {
            log::warn!(
                "{} not found in $PATH on {}",
                command_line.app,
                ssh.to_string()
            );
        }
    }

    // we are not going to use ssh, so we need to check if we need a container
    if !can_ssh {
        if command_line.sudo && !interactive {
            // we're running in non-interactive mode, can't sudo
            needs_container = true;
        } else if !command_line.app_in_path {
            // app not in $PATH, we need a container
            needs_container = true;
        } else if container.is_some() && container.unwrap().force {
            // forced container use
            needs_container = true;
        }
    }

    // wrap the command line in a container if needed
    let command_line = if needs_container {
        let container = match container {
            Some(c) => c,
            None => {
                return Err(anyhow::anyhow!(
                    "container required for function {}",
                    call.function.name
                ))
            }
        };

        log::debug!("using container: {:?}", container);

        // build or pull the image if needed
        container.resolve().await?;

        // wrap the command line
        container.wrap(command_line)?
    } else {
        // keep it as it is
        command_line
    };

    if can_ssh {
        log::warn!(
            "executing (as {}): {}",
            ssh.as_ref().unwrap().to_string(),
            &command_line
        );
    } else {
        log::warn!("executing: {}", &command_line);
    }

    if interactive
        && prompt::ask(
            ">> enter 'y' to proceed or any other key to cancel: ",
            &["y", "n"],
        )? != "y"
    {
        return Ok(openai::CallResultMessage {
            role: "tool".to_string(),
            call_id: call.id.clone(),
            content: "<command execution cancelled by user>".to_string(),
        });
    }

    // finally execute the command line
    let content = if can_ssh {
        // execute via ssh
        ssh.as_ref()
            .unwrap()
            .execute(command_line.sudo, &command_line.app, &command_line.args)
            .await?
    } else {
        // execute locally
        command_line.execute().await?
    };

    Ok(openai::CallResultMessage {
        role: "tool".to_string(),
        call_id: call.id.clone(),
        content,
    })
}

pub(crate) async fn execute(
    ssh: Option<SSHConnection>,
    interactive: bool,
    book: Arc<Book>,
    calls: Vec<openai::Call>,
    max_running_tasks: usize,
) -> anyhow::Result<Vec<openai::CallResultMessage>> {
    let mut futures = Vec::new();
    for call in calls {
        futures.push(tokio::spawn(execute_call(
            ssh.clone(),
            interactive,
            max_running_tasks,
            book.clone(),
            call,
        )));
    }

    let mut results = Vec::new();
    for future_result in futures::future::join_all(futures).await {
        match future_result {
            Ok(result) => match result {
                Ok(result) => results.push(result),
                Err(e) => return Err(anyhow!("error executing call: {:?}", e)),
            },
            Err(e) => log::error!("error joining task: {:?}", e),
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use crate::book::{runtime::ExecutionContext, Function, Page};

    use super::*;
    use std::collections::BTreeMap;
    use std::collections::HashSet;

    fn create_test_book() -> Arc<Book> {
        Arc::new(Book {
            pages: BTreeMap::new(),
            failed_containers: HashSet::new(),
        })
    }

    #[tokio::test]
    async fn test_execute_call() {
        let call = openai::Call {
            id: Some("test_call".to_string()),
            call_type: "function".to_string(),
            function: openai::FunctionCall {
                name: "test_function".to_string(),
                arguments: BTreeMap::new(),
            },
        };

        let mock_page = Page {
            name: "test_page".to_string(),
            description: Some("Test page".to_string()),
            categories: Vec::new(),
            functions: {
                let mut map = BTreeMap::new();
                map.insert(
                    "test_function".to_string(),
                    Function {
                        description: "Test function".to_string(),
                        parameters: BTreeMap::new(),
                        execution: ExecutionContext::CommandLine(vec![
                            "echo".to_string(),
                            "test".to_string(),
                        ]),
                        container: None,
                    },
                );
                map
            },
        };

        let book = Arc::new(Book {
            pages: {
                let mut map = BTreeMap::new();
                map.insert(camino::Utf8PathBuf::from("test_page"), mock_page);
                map
            },
            failed_containers: HashSet::new(),
        });

        let result = execute_call(None, false, 10, book, call).await.unwrap();

        assert_eq!(result.role, "tool");
        assert_eq!(result.call_id, Some("test_call".to_string()));
        assert_eq!(result.content, "test\n");
    }

    #[tokio::test]
    async fn test_execute() {
        let calls = vec![
            openai::Call {
                id: Some("call1".to_string()),
                call_type: "function".to_string(),
                function: openai::FunctionCall {
                    name: "echo1".to_string(),
                    arguments: BTreeMap::new(),
                },
            },
            openai::Call {
                id: Some("call2".to_string()),
                call_type: "function".to_string(),
                function: openai::FunctionCall {
                    name: "echo2".to_string(),
                    arguments: BTreeMap::new(),
                },
            },
        ];

        let mock_page = Page {
            name: "test_page".to_string(),
            description: Some("Test page".to_string()),
            categories: Vec::new(),
            functions: {
                let mut map = BTreeMap::new();
                map.insert(
                    "echo1".to_string(),
                    Function {
                        description: "Echo 1".to_string(),
                        parameters: BTreeMap::new(),
                        execution: ExecutionContext::CommandLine(vec![
                            "echo".to_string(),
                            "test1".to_string(),
                        ]),
                        container: None,
                    },
                );
                map.insert(
                    "echo2".to_string(),
                    Function {
                        description: "Echo 2".to_string(),
                        parameters: BTreeMap::new(),
                        execution: ExecutionContext::CommandLine(vec![
                            "echo".to_string(),
                            "test2".to_string(),
                        ]),
                        container: None,
                    },
                );
                map
            },
        };

        let book = Arc::new(Book {
            pages: {
                let mut map = BTreeMap::new();
                map.insert(camino::Utf8PathBuf::from("test_page"), mock_page);
                map
            },
            failed_containers: HashSet::new(),
        });

        let results = execute(None, false, book, calls, 10).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "test1\n");
        assert_eq!(results[1].content, "test2\n");
    }

    #[tokio::test]
    async fn test_execute_with_non_existent_function() {
        let book = Arc::new(Book {
            pages: {
                let mut map = BTreeMap::new();
                map.insert(
                    camino::Utf8PathBuf::from("test_page"),
                    Page {
                        name: "test_page".to_string(),
                        description: Some("Test page".to_string()),
                        categories: Vec::new(),
                        functions: BTreeMap::new(),
                    },
                );
                map
            },
            failed_containers: HashSet::new(),
        });

        let calls = vec![openai::Call {
            id: Some("call1".to_string()),
            call_type: "function".to_string(),
            function: openai::FunctionCall {
                name: "non_existent_function".to_string(),
                arguments: BTreeMap::new(),
            },
        }];

        let result = execute(None, false, Arc::clone(&book), calls, 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_with_non_existent_command() {
        let book = Arc::new(Book {
            pages: {
                let mut map = BTreeMap::new();
                map.insert(
                    camino::Utf8PathBuf::from("test_page"),
                    Page {
                        name: "test_page".to_string(),
                        description: Some("Test page".to_string()),
                        categories: Vec::new(),
                        functions: {
                            let mut map = BTreeMap::new();
                            map.insert(
                                "non_existent".to_string(),
                                Function {
                                    description: "Non-existent command".to_string(),
                                    parameters: BTreeMap::new(),
                                    execution: ExecutionContext::CommandLine(vec![
                                        "non_existent_command".to_string(),
                                    ]),
                                    container: None,
                                },
                            );
                            map
                        },
                    },
                );
                map
            },
            failed_containers: HashSet::new(),
        });

        let calls = vec![openai::Call {
            id: Some("call1".to_string()),
            call_type: "function".to_string(),
            function: openai::FunctionCall {
                name: "non_existent".to_string(),
                arguments: BTreeMap::new(),
            },
        }];

        let result = execute(None, false, Arc::clone(&book), calls, 10).await;
        assert!(result.is_err());
    }
}
