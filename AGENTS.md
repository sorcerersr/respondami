# AGENTS.md

## Build/Lint/Test Commands
- Run tests with: `uv run pytest` or `uv run python -m pytest`
- Run single test: `uv run pytest tests/test_file.py::test_function_name`
- Lint code: `uv run ruff check .`
- Type check: `uv run basedpyright .`
- Format code: `uv run ruff format .`

## Code Style Guidelines
- Follow PEP 8 with black formatting
- Use type hints for all function parameters and return values
- Import ordering: standard library, external libraries, local imports
- Use snake_case for functions and variables
- Use PascalCase for classes
- Error handling: Use specific exceptions and log errors appropriately
- Docstrings: Use Google-style docstrings for public APIs

## Development Setup
- Use uv for package management
- Create a virtual environment with: `uv venv`
- Activate the virtual environment: `source .venv/bin/activate` (Linux/macOS) or `.venv\Scripts\activate` (Windows)
- Install dependencies with: `uv sync`