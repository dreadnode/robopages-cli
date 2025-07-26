from os import getenv
from typing import Dict, List, Optional, Type
import requests

from langchain_core.tools import BaseTool, ArgsSchema
from pydantic import BaseModel, Field, create_model

_SERVER_URL = getenv("ROBOPAGES_SERVER", "http://127.0.0.1:8000")

class RoboPagesTool(BaseTool):
    name: str
    description: str
    parameters: List[Dict]
    args_schema: Optional[ArgsSchema]

    __baseURL: str = _SERVER_URL

    def _run(self, *args, **kwargs):
        process_url = f"{self.__baseURL}/process"
        headers = {"Content-type": "application/json"}

        payload = [
            {
                "type": "function",
                "function": {
                    "name": self.name,
                    "arguments": kwargs
                }
            }
        ]

        response = requests.post(
            url=process_url,
            headers=headers,
            json=payload
        )
        return response.json()[0]

class RoboPagesOutput(BaseModel):
    tool: str = Field(description="The tool that was used")
    parameters: Dict = Field(description="The parameters for the requested tool")

class RoboPages:
    def __init__(self, server_url: str = None):
        """Initialize RoboPages with an optional base URL override."""
        self.__server_url: str = server_url if server_url else _SERVER_URL
        self.__server_url += "/?flavor=rigging"
        self.tools: List[RoboPagesTool] = []
        self.RoboPagesOutput = RoboPagesOutput

    def __get_tools(self):
        response = requests.get(self.__server_url)
        response.raise_for_status()
        return response.json()

    def __create_tools(self) -> List[RoboPagesTool]:
        """Create LangChain Tools based on the functions from the root endpoint."""
        functions = self.__get_tools()
        self.tools = []

        for item in functions:
            for func in item["functions"]:
                name = func["name"]
                description = func["description"]
                parameters =  func["parameters"]

                #Building arg fields for pydantic
                args = {}
                for param in parameters:
                    args[param["name"]] = (param["type"], Field(description=param["description"]))

                tool = RoboPagesTool(
                    name= name,
                    description= description,
                    parameters= parameters,
                    args_schema= create_model( f"{name}_schema", **args),
                )
                self.tools.append(tool)

        return self.tools

    def get_tools(self) -> List[RoboPagesTool]:
        """Return the list of created tools, fetching and creating tools if needed."""
        if not self.tools:
            self.__create_tools()
        return self.tools

    def get_tool(self, name: str) -> RoboPagesTool | None:
        """Retrieve a specific tool by its name, fetching and creating tools if needed."""
        if not self.tools:
            self.__create_tools()
        for tool in self.tools:
            if tool.name == name:
                return tool
        return None

    def filter_tools(self, filter_string: str) -> List[RoboPagesTool] | None:
        """Retrieve a set of tools with the filter_string in the name, fetching and creating tools if needed."""
        output: List[RoboPagesTool] = []
        if not self.tools:
            self.__create_tools()
        for tool in self.tools:
            if filter_string.lower() in tool.name:
                output.append(tool)
        if output:
            return output
        else:
            return None


if __name__ == "__main__":
    # Testing RoboPagesTool with default http_get function parameters
    print("++ Test tool 'http_get':")
    RoboPagesTool_test_name = "http_get"
    RoboPagesTool_test_description = "Perform an HTTP GET request to a given URL."
    RoboPagesTool_test_parameters =[
        {
            "description": "The URL to perform the GET request on.",
            "examples": "",
            "name": "url",
            "type": "str"
        },
        {
            "description": "An optional, NON EMPTY User-Agent string to use for the request.",
            "examples": "",
            "name": "user_agent",
            "type": "str"
        }
    ]
    http_get = RoboPagesTool(
        name=RoboPagesTool_test_name,
        description=RoboPagesTool_test_description,
        parameters=RoboPagesTool_test_parameters,
        args_schema=create_model(
            "http_get",
            url=(str, Field(description="The URL to perform the GET request on.")),
            user_agent=(str, Field(description="An optional, NON EMPTY User-Agent string to use for the request."))
        )
    )
    http_get_tool_call = http_get.invoke({
        "url": "http://example.com",
        "user_agent": "RoboPages"
    })
    if http_get_tool_call[0:15] == "<!doctype html>":
        print(f"\033[32mPassed!\033[0m")
    else:
        print(f"\033[91mFailed!\033[0m")

    print("++ Test Class RoboPages:")
    print("--- Create Tools Function")
    rb = RoboPages()
    robo_tools = []
    robo_tools = rb.get_tools()
    if robo_tools:
        print(f"\033[32mPassed!\033[0m")
    else:
        print(f"\033[91mFailed!\033[0m")

    print("--- Get Tool Function")
    rb = RoboPages()
    robo_tool = []
    robo_tool = rb.get_tool(name= "http_get")
    if robo_tool:
        print(f"\033[32mPassed!\033[0m")
    else:
        print(f"\033[91mFailed!\033[0m")
    print("Done!")

    print("--- Filter Tool Function")
    rb = RoboPages()
    robo_tool = []
    robo_tool = rb.filter_tools(filter_string="http_get")
    if robo_tool:
        print(f"\033[32mPassed!\033[0m")
    else:
        print(f"\033[91mFailed!\033[0m")
    print("Done!")
