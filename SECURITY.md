# Security Policy

`claudex` works with local coding-agent transcripts. Those transcripts and generated handoff files may contain private prompts, source code, filesystem paths, command output, credentials, or other sensitive data.

## Supported Versions

`claudex` is currently pre-1.0. Security fixes will target the latest `main` branch unless a release policy is added later.

## Reporting a Vulnerability

Please do not open a public issue with secrets, private transcripts, or exploit details.

If no private reporting channel is listed for this repository yet, open a public issue that says you have a security concern without including sensitive details. A maintainer can then arrange a private channel.

## Handling Test Data

Do not add real Claude Code or Codex transcripts to the repository. Use small, synthetic, sanitized fixtures that preserve only the event shape needed for a test.

## Local Data

By default, `claudex` writes handoff files under:

```text
~/.handoffs
```

Review handoff files before sharing them. They are designed to be readable, not automatically sanitized.
