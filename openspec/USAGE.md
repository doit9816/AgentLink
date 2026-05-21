# OpenSpec + Codex Usage

AgentLink uses OpenSpec for changes that affect behavior, architecture, protocol shape, config, storage, or cross-layer UI/backend flows.

For small typo fixes or obvious one-line bugs, edit directly.

## Commands

Use these in Codex from the repository root:

```text
/opsx:propose "add lark websocket diagnostics"
/opsx:apply
/opsx:archive
```

Useful CLI checks:

```powershell
openspec list
openspec list --specs
openspec status --change "<change-name>"
openspec validate "<change-name>"
openspec validate "<spec-id>"
```

## Workflow

1. Start with `/opsx:propose "..."` for any non-trivial feature or behavior change.
2. Review the generated files under `openspec/changes/<change-name>/`.
3. Ask Codex to apply the approved change with `/opsx:apply`.
4. Run the checks required by `AGENTS.md` and the change tasks.
5. Archive the completed change with `/opsx:archive`.

Keep `AGENTS.md` as the source of local development constraints. OpenSpec config mirrors the most important rules so proposals and tasks inherit them.
