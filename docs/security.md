# Security

Respondami runs an LLM agent with tool access. By default, the agent has **full access to the filesystem and shell** of the host process. There are no sandbox restrictions built into the application.

## Threat Model

The agent can:

- Read any file the process can access
- Write to any writable path
- Execute arbitrary shell commands via the `bash` tool
- Access network resources (provider API, any URL via bash)

When running **untrusted or public models**, a malicious or misbehaving model could:

- Exfiltrate sensitive files (`.env`, SSH keys, credentials)
- Modify project files with malicious content
- Install packages or scripts on the host
- Make network requests from the host

## Mitigation: Run in a Sandbox

The only effective mitigation is to constrain the agent's environment. Respondami does not enforce sandboxing — the user must configure it.

### OCI Container (Recommended)

Run Respondami inside a container with minimal privileges:

```bash
podman run --rm -it \
  -v "$(pwd):/workspace:ro" \
  -w /workspace \
  respondami:latest
```

Key options:

- `--rm` — destroy container on exit
- `-v ...:ro` — mount workspace read-only (or omit `:ro` for write access)
- `--network none` — block all network access (use if provider runs on host)
- `--cap-drop=ALL` — drop all Linux capabilities
- `--read-only` — make the container filesystem read-only

### Firejail

[Firejail](https://firejail.wordpress.com/) provides sandboxing without a container:

```bash
firejail --whitelist=/home/user/project --net=none respondami
```

### VM

For maximum isolation, run Respondami in a vm or microvm with no shared filesystem outside the project directory.

## Path Traversal (Won't Fix)

Read, write, and edit tools accept paths relative to the working directory with no traversal validation. The `bash` tool has no restrictions at all.

Adding path validation to file tools alone provides **no real security** — the agent can bypass it entirely via bash (`cat /etc/passwd`, `cp /etc/shadow ./`). Partial restrictions would create a false sense of security.

The effective mitigation is the same: run in a sandbox.

## Provider Security

The provider configuration file may contain API keys or authentication credentials. Store it with restrictive permissions:

```bash
chmod 600 ~/.config/respondami/config.json
```
