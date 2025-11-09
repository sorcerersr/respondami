# Respondami - MCP Server

## Introducton

Respondami is a self hosted mcp server to provide AI coding assistants with up to date library documentations.

## Motivation

There are several other MCP server implementations with the same goal to provide library/package documentation to AI coding assistants. This is just my own take for an implementation as I'm not happy with exiting solutions for one or the other reason.

## Features

* Open-Source
* self host
* support for multiple sources (documentation, code examples, etc.)


## Using model-inspector

```ALLOWED_ORIGINS=http://127.0.0.1:6274 HOST=127.0.0.1 uv run mcp dev main.py```
