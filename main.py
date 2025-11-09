from mcp.server.fastmcp import FastMCP

# Create an MCP server
mcp = FastMCP(Example Service")


@mcp.tool()
def get_hello(name: str) -> str:
    """Get hello `name` message from server."""
    return f"Hello {name}. Hope you are having a great day!"


# Run the server
if __name__ == "__main__":
    mcp.run()
