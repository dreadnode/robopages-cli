# Robopages CLI and Function Calling Server

[Robopages are YAML based files](https://github.com/dreadnode/robopages) for describing tools to large language models (LLMs). They simplify the process of defining and using external tools in LLM-powered applications. By leveraging the `robopages-cli` function calling server, developers can avoid the tedious task of manually writing JSON declarations for each tool. This approach streamlines tool integration, improves maintainability, and allows for more dynamic and flexible interactions between LLMs and external utilities.

Pages are loaded by default from the `~/.robopages/` directory (or any folder set in the `ROBOPAGES_PATH` environment variable), see the `https://github.com/dreadnode/robopages` repository for examples.

## Build Docker image

To build the Docker image for the tool, run:

```bash
docker build . -t robopages
```

Optionally, you can create a bash alias like so:

`alias robopages='docker run -v ~/.robopages:/root/.robopages -p 8080:8080 robopages'`

## Build from source

Alternatively you can build the project from source, in which case you'll need to have Rust and Cargo [installed on your system](https://rustup.rs/) and clone this repository.

To build the project:

```bash
cargo build --release
```

The compiled binary will be available in the `target/release` directory. You can run it directly or add it to your system's PATH:

```bash
# Run directly
./target/release/robopages

# Or, copy to a directory in your PATH (e.g., /usr/local/bin)
sudo cp target/release/robopages /usr/local/bin/
```

## Usage

This project consists of a CLI for creating, viewing and serving robopages as a REST API.

### CLI

Install robopages:

```bash
# install https://github.com/dreadnode/robopages to ~/.robopages/robopages-main
robopages install

# install a custom repository
robopages install --source user/repo

# install from a local archive
robopages install --source /path/to/archive.zip
```

View installed robopages:

```bash
robopages view
```

Create a robopage with the preferred template:

```bash
# create with the basic template, will run the command in the current shell
robopages create --name my_first_page.yml --template basic

# create with the docker-image template, will use a docker image to run the command
robopages create --name my_first_page.yml --template docker-image

# create with the docker-build template, will build a docker image to run the command
robopages create --name my_first_page.yml --template docker-build
```

Start the REST API:

```bash
# this will pre build and pull all containers
robopages serve

# this will build or pull containers on demand
robopages serve --lazy
```

Execute a function manually without user interaction:

```bash
robopages run --function nikto_scan --auto
```

You can also define variables to be used in the function call:

```bash
robopages run -F httpx_tech_detect -A --define target=www.example.com
```

Repeat for multiple variables:

```bash
robopages run -F function_name -A -D target=www.example.com -D foo=bar
```

### Using with LLMs

The examples folder contains integration examples for [Rigging](/examples/rigging_example.py), [OpenAI](/examples/openai_example.py), [Groq](/examples/groq_example.py), [OLLAMA](/examples/ollama_example.py) and [Nerve](/examples/nerve.md).
