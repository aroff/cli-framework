# Agent chat rollout plan

Work is tracked in **at most three GitHub issues per repository** (consolidated checklists). Granular AK/CF/APP IDs below are **reference labels** inside those issues, not separate deploy tickets.

## Implementation order

1. **[goaikit/aikit#28](https://github.com/goaikit/aikit/issues/28)** — Phase 1 (AK-01, AK-02, AK-06, AK-07): embedded API, output contract, `aikit run` parity, tests.
2. **[goaikit/aikit#29](https://github.com/goaikit/aikit/issues/29)** — Phase 2 (AK-03, AK-04, AK-05, AK-08): `HostToolProvider`, agent host tools, docs, optional skills.
3. **[aroff/cli-framework#49](https://github.com/aroff/cli-framework/issues/49)** — Phase 1 (CF-01..CF-05, CF-08, CF-09): `chat` feature, CLI, host tools, `AppContext`, ask→chat, async. **CF-04** blocked until aikit **#29**.
4. **[aroff/cli-framework#50](https://github.com/aroff/cli-framework/issues/50)** — Phase 2 (CF-06, CF-07, CF-10, CF-11): stdio MCP, gates, security docs, CI.
5. **[aroff/product-cli#11](https://github.com/aroff/product-cli/issues/11)** — Adoption (APP-01..APP-03): features, gates, user docs. Blocked until cli-framework **#49** / **#50** (or agreed MVP) is released.

`cli-framework` should not ship **CF-04** until **aikit#29** lands (HostToolProvider + agent). **CF-02** benefits from **aikit#28** (AK-01, AK-02).

## Reference: original requirement IDs (checklists live in GitHub issues)

### goaikit/aikit

| IDs | Topic |
|-----|--------|
| AK-01, AK-02, AK-06, AK-07 | [#28](https://github.com/goaikit/aikit/issues/28) |
| AK-03, AK-04, AK-05, AK-08 | [#29](https://github.com/goaikit/aikit/issues/29) |

### aroff/cli-framework

| IDs | Topic |
|-----|--------|
| CF-01 .. CF-05, CF-08, CF-09 | [#49](https://github.com/aroff/cli-framework/issues/49) |
| CF-06, CF-07, CF-10, CF-11 | [#50](https://github.com/aroff/cli-framework/issues/50) |

### aroff/product-cli

| IDs | Topic |
|-----|--------|
| APP-01 .. APP-03 | [#11](https://github.com/aroff/product-cli/issues/11) |

## GitHub projects

| Board | URL |
|--------|-----|
| AIKit (aikit issues) | https://github.com/orgs/goaikit/projects/1 |
| cli-framework (cli-framework + product-cli rollout) | https://github.com/users/aroff/projects/13 |

Earlier per-ID issues were **deleted** from GitHub so deploy automation stays on **#28, #29, #49, #50, #11** only.
