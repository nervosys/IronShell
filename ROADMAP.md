# IronShell Roadmap

> Current release status, 2026-11-20: crate **`iron-shell` 14.0.5** and
> **`iron-shell-lsp` 14.0.5** are on crates.io. The binary is `ish`, and `ae`
> is installed as an alias. The 14.0.x releases are tagged `v14.0.0` to
> `v14.0.5`; the GitHub release `v14.0.5` is marked Latest. The
> [completion plan](docs/COMPLETION_PLAN.md) and the
> [productization plan](docs/PRODUCTIZATION_PLAN.md) are historical. Their open
> items are carried over in the section "Carried over from the completion and
> productization plans" below.

## 14.0.0 preparation notes (2026-10, historical)

These paragraphs describe the state on 2026-10-19 and are kept as a record.
Where they say something "remains open", check the carried-over section below.

The user expanded the 13.0.0 implementation scope on 2026-10-02 to all unsupported
builtins. Scheduler mutation adapters, DQN weight training and native TCP socket lifecycle now have working
implementations and targeted tests. Native Ed25519 key generation, signing and
verification now replace signature placeholders; non-Unix RSA/EC generation remains open.
LCOV coverage summaries and completed libtest JSON outcome reports are now implemented
as explicit, pure report operations. Rust function signature changes and parameter
addition/removal now return bounded, syntactic source edits; caller compilation
and call-site updates remain explicit. Variable and explicitly typed constant
extraction now edit a function's top-level tail expression; caller compiler and
semantic checks remain necessary. Rust `refactor_inline` now inlines one
single-use top-level `let` into an immediately following, unconditionally
evaluated by-value use, and rejects reorderings that could change effects or
drop timing. Rust `refactor_rename` now renames identifiers across one supplied
source file syntactically, `refactor_extract_function` moves a top-level statement
range into a private function, and `refactor_move_to_file` moves one top-level item
between supplied texts; none updates other files or adds imports, and caller
compilation remains necessary. `session_undo`, `session_redo`, `session_history` and
`session_end` now operate on the reversible-session journal, and `test_watch` reruns
an argv command on polled file changes with bounded deadlines and whole-process-tree
kill on timeout; test_watch is classified Exec. Exact warning-code filtering of supplied
rustc/Cargo JSON diagnostics is implemented without changing compiler policy.
`fs_watch` and `fs_unwatch` now provide native, process-local filesystem watches with
explicit handles, bounded event buffers and ReadLocal effects. `crypto_password_verify`
verifies SHA-crypt `$5$` and `$6$` hashes per Drepper's specification and refuses them
under IRONSHELL_FIPS; `nc_listen` is a one-shot, bounded loopback listener, and ten platform
branches now report coded errors instead of silent failures.
Disposable-platform scheduler validation
remains open. Linux backends for accounts, groups, ACLs, mounts, systemd services, sudo_exec and apt
are implemented and were verified as root in a disposable Debian distro; Windows and macOS
refuse them explicitly, so those platform backends and other unsupported operations remain
implementation blockers. The completion plan separates those gaps
from legitimate platform, policy and transaction constraints. Refusal contracts
alone do not close this implementation scope.

## Historical roadmap — reference only

The February snapshot below is preserved as history. Its “Complete” labels,
versions, counts and distribution claims are historical assertions, not current
release validation or independently verified registry status. Use the completion
plan for outstanding 14.0.0 work and the release checklist before publication.


> **Last Updated:** February 20, 2026

This document tracks the development progress of IronShell, the world's first agentic shell with typed functional pipelines and multi-modal AI.

---

## Progress Overview

| Category          | Status     | Details                                   |
| ----------------- | ---------- | ----------------------------------------- |
| Core Language     | ✅ Complete | AST-based, Hindley-Milner                 |
| Type System       | ✅ Complete | Full inference                            |
| Builtins Library  | ✅ Complete | 1,100+ functions, 108 modules             |
| AI Integration    | ✅ Complete | 9 providers, unified stack, streaming     |
| Agent Framework   | ✅ Complete | Single + swarm + A2A                      |
| AI Ontology       | ✅ Complete | 1,100+ builtins, 108 modules discoverable |
| TUI Interface     | ✅ Complete | Tabs, chat, dashboard                     |
| Theme System      | ✅ Complete | 38 themes                                 |
| Config System     | ✅ Complete | XDG-compliant                             |
| Plugin System     | ⚠️ Partial  | TOML metadata registration; script builtins refused |
| Standard Library  | ✅ Complete | 7 modules (lib/)                          |
| Performance       | ✅ Complete | 5 benchmark suites                        |
| Test Coverage     | ✅ Complete | 6,701 `#[test]` functions (see Metrics)   |
| Documentation     | ✅ Complete | Comprehensive                             |
| Publishing        | ✅ Complete | crates.io `iron-shell` 14.0.5             |
| WASM Support      | ✅ Complete | Browser REPL ready                        |
| Enterprise        | ⚠️ Partial  | RBAC, Audit; sso_auth refuses             |
| LSP Server        | ✅ Complete | tower-lsp, crates.io                      |
| VS Code Extension | ✅ Complete | Marketplace published                     |
| MCP Protocol      | ✅ Complete | 130+ tools, HTTP server                   |
| Agent API         | ✅ Complete | 27 provider schema formats                |
| Distribution      | ⚠️ Partial  | crates.io, GitHub Releases, Homebrew formula in repo, PyPI; no npm package |
| CI/CD             | ✅ Complete | GitHub Actions, CLA check                 |
| Licensing         | ✅ Complete | AGPL-3.0 + commercial                     |

---

## Version History

### v1.2.0 (historical checkpoint) — February 2026
- [x] **Streaming support** — `chat_stream()` implemented for all 4 providers: OpenAI (SSE), Anthropic (SSE with event types), Google Gemini (SSE), Ollama (NDJSON); `StreamChunk`/`StreamDelta` types; `supports("streaming")` returns true on all providers
- [x] **Cost-based routing** — added `CostUnder { max_cost_per_1k }` and `LatencyUnder { max_ms }` conditions to `RoutingCondition` enum (13 total variants)
- [x] **Latency tracking** — extended `ProviderStats` with `min_latency_ms`, `max_latency_ms`, `p95_latency_ms`, `total_cost_usd`, and `last_request_epoch` fields
- [x] **Structured output fix** — converted 9 remaining raw-text builtins to typed Records/Arrays: `net_whois`, `at_list`, `pkg_sources`, `pkg_history`, `capabilities`, `strace`, `sar`, `dmesg` fallback, `fdisk_list` fallback
- [x] **Platform module fix** — removed triple-registration of `platform` module in `all_modules()`
- [x] **VS Code Web extension** — full scaffold with syntax highlighting, eval command, hover/completion providers, Agent API integration
- [x] **JupyterLab extension** — Jupyter kernel with Agent API proxy, code completion, `.ish` file type registration
- [x] **Load balancing** — `ai_set_load_balancing()` and `ai_load_balancing()` builtins with 5 strategies: round-robin, least-latency, least-requests, weighted, adaptive; `LoadBalancer` in `ProviderRegistry` with lock-free `AtomicU64` counters
- [x] **Local inference** — `local_inference` module with Candle and ONNX Runtime backends behind feature flags; builtins `ai_local_backends()`, `ai_local_load()`, `ai_local_generate()`, `ai_local_embed()`, `ai_local_unload()`
- [x] **Transpiler improvements** — PowerShell cmdlet mappings expanded from 42 → 100+ with dot-notation alignment; 12 tests added for Bash transpiler (previously zero); Zsh `rehash`/`emulate` stubs fixed

### v0.3.1 — February 2026
- [x] CLI tool wrappers (tools 1-150): 200+ builtins wrapping common CLI tools — process management (tmux, screen, htop), disk/filesystem (ncdu, duf, dust), debugging (gdb, valgrind, strace, ltrace), binary inspection (objdump, readelf, nm, ldd, strings, file), networking (iperf3, nmap, mtr, dig, nc), modern CLI replacements (bat, fd, rg, sd, zoxide, fzf, jq, yq, delta, hyperfine, tokei, just, direnv), version/runtime managers (asdf, mise, nvm, pyenv, rbenv), language toolchains (cargo, rustup, go, node, npm, pnpm, yarn, bun, deno, uv, pipx, poetry, pytest), dev tools (gh, glab, pre-commit, make, cmake, ninja, nodemon), containers (buildah, skopeo, trivy, podman-compose, docker-compose), linters/formatters (shellcheck, shfmt, black, ruff, eslint, prettier, clippy, rustfmt, golangci-lint)
- [x] Cross-platform OS commands: 219 new builtins for containers, Kubernetes, VMs/hypervisors, cloud/IaC, remote access, security, and monitoring
- [x] Cross-platform consistency audit: 100+ `cfg` blocks across 6 files reviewed, 40+ functions fixed for OS parity
- [x] Structured output: converted 20+ builtins from raw text to typed Records/Arrays (net_ping, net_traceroute, net_arp, net_route, net_ports, net_connections, net_bandwidth, svc_logs, cron_list, pkg_list, pkg_search, pkg_info, pkg_files, pkg_owner, sys_uptime, sys_info, proc_children, startup_list, and more)
- [x] PascalCase normalization: all Windows builtins using `json_to_value()` now return consistent lowercase field names (10 sites fixed)
- [x] macOS support: added missing implementations for 15+ builtins (tcpdump, ss_info→netstat, sys_uptime→sysctl, sys_info→sw_vers, net_bandwidth→netstat -ib, dmesg, fdisk_list→diskutil, proc_children→pgrep, startup_list→launchctl)
- [x] Cfg gate fixes: `dd_copy` and `chroot` widened from Linux-only to Unix (not(windows)), since both tools ship natively on macOS
- [x] Unified 20 builtins to use cross-platform Rust crates (sysinfo, chrono, sha2, md5, which) instead of OS-specific commands
- [x] Published to crates.io as `aethershell v0.3.1`
- [x] Published `aethershell-lsp v0.2.0` to crates.io
- [x] VS Code extension v0.3.1 on Marketplace (`admercs.ironshell`)
- [x] License: AGPL-3.0-or-later with commercial dual-license
- [x] Contributor License Agreement (CLA) + CI enforcement
- [x] All 88 compiler warnings fixed (zero-warning build)
- [x] GitHub Releases with Windows + Linux binaries
- [x] Linguist submission package prepared (samples, grammar, guide)
- [x] Crate package optimized (10.1 MiB → 733 KiB compressed)
- [x] README badges: CI, crates.io version, downloads, VS Code, license, stars
- [x] Linux packages (.deb/.rpm) — Cargo.toml metadata + CI job
- [x] LangChain HTTP Agent API tools (`AgentAPIClient` + 3 tool classes)
- [x] System AI assistant (`ish assist`) — interactive/execute/context/suggest modes
- [x] AI discoverability metadata (llms.txt, AGENTS.md, OpenAPI, plugin manifest)
- [x] PyPI distribution for Python SDK (pyproject.toml, CI workflow)
- [x] Context-aware command suggestions and NL transpilation
- [x] Windows Terminal custom profile

### v0.3.0 — January 2026
- [x] Implicit match scrutinee in lambda bodies
- [x] Bash transpiler improvements
- [x] Python SDK (integrations/python/)
- [x] GitHub Actions CI/CD (ci.yml, release.yml, security-audit.yml)
- [x] CLA check workflow

### v0.2.0 — January 2026
- [x] Plugin system with dynamic TOML loading
- [x] 5 benchmark suites (parser, eval, pipeline, builtin, MCP)
- [x] Distributed computing builtins
- [x] Advanced AI (RAG, Knowledge Graphs, Semantic Caching)
- [x] Enterprise features (RBAC, Audit, SSO, Compliance)
- [x] LSP server (iron-shell-lsp crate)
- [x] VS Code extension v0.2.0 with hover, symbols, folding
- [x] Error recovery with multi-error reporting
- [x] Async/await syntax
- [x] Debugging tools (debug, trace, assert, inspect)
- [x] Platform detection and feature flags
- [x] Conditional compilation (#[cfg(...)])
- [x] Standard library (lib/)
- [x] Module visibility (pub/private, export)
- [x] Package management (import, ironshell.toml)
- [x] WASM support with browser REPL
- [x] Distribution: Homebrew, npm, browser extension

### v0.1.x — December 2025
- [x] Core language: AST evaluator, Hindley-Milner type inference
- [x] Typed pipelines, first-class lambdas, pattern matching
- [x] 430+ builtins across functional, string, array, math, file, OS, container, VM, cloud, security categories
- [x] AI integration: multi-provider, multi-modal (images, audio, video)
- [x] Agent framework: autonomous agents with tool access
- [x] Agent swarms with coordinator patterns
- [x] TUI interface with chat, agents, media viewer
- [x] MCP protocol: 130+ tools, HTTP server mode
- [x] Agent API: OpenAI/Claude/Gemini function calling schemas

---

## Completed Features (Full Detail)

### Core Language
- [x] AST-based evaluation engine
- [x] Hindley-Milner type inference
- [x] Typed pipelines (structured Value types, not text)
- [x] First-class functions and lambdas (`fn(x) => x * 2`)
- [x] N-ary lambda support (3+ parameters)
- [x] Zero-parameter lambdas
- [x] Pattern matching with guards
- [x] String interpolation (`"Hello ${name}"`)
- [x] Record and array literals
- [x] Try/catch/throw error handling
- [x] Async/await syntax
- [x] Implicit match scrutinee in lambdas
- [x] Conditional compilation (`#[cfg(...)]`)

### Module & Package System
- [x] Module visibility (pub/private, export)
- [x] Import syntax (path, namespaced, selective, aliased, registry)
- [x] Package manifest (ironshell.toml)
- [x] Module cache with cycle detection
- [x] Standard library (7 modules: prelude, math, string, collection, functional, io, test_stdlib)

### Builtins (1,100+ functions in 108 modules)
- [x] `file`, `sys`, `proc`, `fs`, `net`, `http`, `gui`, `web`
- [x] `crypto`, `db`, `svc`, `cron`, `archive`, `user`, `perm`, `pkg`
- [x] `hw`, `clip`, `input`, `ai`, `agent`, `math`, `str`, `arr`, `json`
- [x] `mcp`, `shell`, `platform`, `a2ui`, `a2a`, `nanda`
- [x] `rbac`, `audit`, `sso`, `cluster`, `nn`, `evo`, `rl`
- [x] `docker`, `podman`, `container` — Container management
- [x] `k8s`, `helm` — Kubernetes orchestration
- [x] `vm`, `hyperv`, `virsh`, `wsl`, `qemu`, `lxc` — VM/hypervisor management
- [x] `terraform`, `ansible`, `pulumi`, `vagrant`, `packer` — Cloud/IaC
- [x] `ssh`, `scp`, `rsync`, `rdp` — Remote access
- [x] `firewall`, `selinux`, `apparmor`, `ssl` — Security
- [x] `monitor`, `perf`, `netstat` — System monitoring
- [x] `tmux`, `screen`, `valgrind`, `gdb`, `objdump`, `readelf` — Process/debug/binary tools
- [x] `zoxide`, `just`, `direnv`, `asdf`, `mise` — Modern CLI & runtime managers
- [x] `uv`, `pipx`, `poetry`, `cargo`, `rustup`, `go` — Language toolchains
- [x] `node`, `npm`, `pnpm`, `yarn`, `bun`, `deno` — JS/TS ecosystem
- [x] `gh`, `glab`, `pre_commit` — Dev tools
- [x] `buildah`, `skopeo`, `trivy`, `ruff` — Container & lint tools
- [x] `iperf3`, `nc`/`netcat` — Network testing
- [x] Cross-platform consistency: all builtins return structured typed output (Records/Arrays), consistent field names across Windows/Linux/macOS

### AI & Agents
- [x] Multi-provider: OpenAI, Anthropic, Ollama, vLLM, compatible
- [x] Model URIs: `openai:gpt-4o`, `ollama:llama3`, `compat:mixtral`
- [x] Multi-modal: images, audio, video
- [x] Autonomous agents with tool access and step limits
- [x] Agent swarms with coordinator patterns
- [x] Agent API server (`ish agent serve`)
- [x] OpenAI / Claude / Gemini function calling schemas
- [x] RAG, Knowledge Graphs, Semantic Caching
- [ ] Fine-tuning management (finetune_start refuses with E_UNIMPLEMENTED: no training backend; no job is created)

### Protocols
- [x] MCP (Model Context Protocol): 130+ tools, 27 categories, HTTP server
- [x] A2A (Agent-to-Agent): inter-agent messaging
- [x] A2UI (Agent-to-User Interface): notifications, progress, confirmations
- [x] NANDA: consensus protocol

### TUI
- [x] Interactive terminal UI with tabs
- [x] Chat interface with context
- [x] Agent dashboard for swarm control
- [x] Media viewer for images/audio/video
- [x] 38 built-in color themes

### Enterprise
- [x] RBAC (role_create, role_grant, check_permission, etc.)
- [x] Audit logging (audit_log, audit_query, audit_export)
- [ ] SSO integration (sso_init records configuration; sso_auth refuses with E_UNIMPLEMENTED, so no session is started)
- [x] Compliance reporting (compliance_check never reports compliant while a control is unverified)
- [ ] Distributed computing: the cluster node registry works, but job scheduling is not implemented (job_submit refuses with E_UNIMPLEMENTED)

### ML Built-ins
- [x] Neural networks (nn_create, nn_forward)
- [x] Evolutionary algorithms (evo_population, evo_evolve)
- [x] Reinforcement learning (Q-Learning, DQN, Actor-Critic)
- [x] NEAT implementation
- [x] Consensus networks

### Tooling & Distribution
- [x] VS Code extension (`admercs.ironshell`) — syntax, snippets, hover, LSP
- [x] LSP server (`iron-shell-lsp`) — completions, diagnostics, symbols, go-to-def
- [x] TextMate grammar + Markdown injection grammar
- [x] Homebrew formula
- [ ] npm package: none has ever been published (`@nervosys/aethershell` returns 404 on the registry, checked 2026-11-20)
- [x] Python SDK (`integrations/python/`)
- [x] WASM support with browser REPL
- [x] GitHub Actions CI/CD (test, release, security audit, CLA)

---

## In Progress

### Q1 2026 items, status on 2026-11-20
- [ ] **GitHub Linguist submission** — Grammar, samples, and submission guide ready; blocked on ecosystem threshold (~200 repos with `.ish` files). Not rechecked; carried over below.
- [x] **VS Code extension marketplace update** — superseded. The extension in `editors/vscode` is at 1.6.1 (publisher `admercs`, name `ironshell`), and the completion plan recorded `admercs.aethershell` 1.6.1 on the Marketplace and Open VSX on 2026-10-27. The listing under the new name was not checked.

---

## Planned Features

### Q2 2026 — Polish & Ecosystem
- [x] Linux packages (.deb, .rpm)
- [x] Windows Terminal integration (custom profile)
- [x] VS Code Web extension
- [x] PyPI distribution for Python SDK
- [x] LangChain tool integration
- [x] JupyterLab extension
- [ ] Publish Linguist PR (pending ecosystem growth)
- [x] AI discoverability (llms.txt, AGENTS.md, ai-plugin.json, OpenAPI, IDE rules)

### Q3 2026 — System AI Assistant
- [x] System AI assistant mode (`ish assist`)
- [x] Context-aware command suggestions
- [x] Natural language → IronShell transpilation
- [x] Conversation memory and session persistence
- [x] Proactive monitoring and alerting
- [x] Shell transpilers: Bash, Zsh, PowerShell → IronShell with block accumulation
- [x] CLI flags `--zsh`/`-z`, `--pwsh`/`-p` and auto-detection by file extension

### Q4 2026 — Cloud & Scale
- [ ] Cloud platform (hosted IronShell instances) — `cloud_deploy`, `cloud_instance_create`, `cloud_instance_status` and `cloud_instance_connect` refuse with E_UNIMPLEMENTED: no provider client is wired in
- [x] Remote REPL — `repl_serve` and `repl_connect` run over a loopback TCP line protocol with a per-listener token, agent mode only. `repl_broadcast` refuses with E_UNIMPLEMENTED: no peer set is held
- [ ] Team workspaces with shared agents — `workspace_sync` refuses with E_UNIMPLEMENTED: there are no remote peers
- [x] Marketplace for community plugins and agents — `marketplace` module: 8 builtins (publish, search, install, uninstall, list, info, rate, update)
- [x] Telemetry and usage analytics (opt-in) — `telemetry` module: 6 builtins (enable, disable, status, report, events, reset)

### v1.0.0 — Production Release
- [x] Stability freeze and backward compatibility guarantees
- [x] Comprehensive API documentation (rustdoc)
- [x] Security audit (internal: 11 findings, all CRITICAL/HIGH remediated)
- [x] Long-term support (LTS) commitment (docs/LTS.md)
- [x] Migration guide from bash/zsh/PowerShell
- [x] Shell transpilers for seamless adoption (Bash, Zsh, PowerShell)

### v1.1.0 — AI Provider Unification & Ontology

**Goal:** Make every builtin, module, and AI provider machine-discoverable through a unified ontology, and consolidate the three parallel provider systems into one.

#### Provider Architecture
- [x] **Unify provider routing** — expanded `complete_sync_router()` in `ai.rs` to support 19+ providers via OpenAI-compatible endpoints with `complete_via_compat()` helper and `provider_base_url()` registry; model URI syntax `provider:model` supported
- [x] **Implement `LLMProvider` trait** for all 19 declared providers — 4 concrete implementations (OpenAI, Anthropic, Google, Ollama) dispatched from `create_provider()` factory; all 14 trait methods implemented; bridge.rs provides `complete_with_provider()`, `chat_with_provider()`, `embed_with_provider()`, `UniversalBackend` adapter
- [x] **Activate provider routing & fallback** — `complete_via_registry()` wired to `complete_sync_router()` via `IRONSHELL_AI_ROUTER=registry`; `ProviderRegistry.route()` evaluates composable `RoutingRule`/`RoutingCondition` (Always, HasTools, HasVision, ModelPattern, TokenCountOver, TimeRange); fallback across providers; builtins `ai_providers()`, `ai_routes()`, `ai_add_route()`, `ai_set_default()`, `ai_registry_stats()`
- [x] **Add Ollama to `ai_api` server** — OllamaProvider with chat, embeddings, model listing, and auto-pull support
- [x] **LM Studio explicit detection** — auto-detect at `:1234` (OpenAI-compatible), `lmstudio:` URI scheme, LMStudioProvider in ai_api
- [x] **Model cost tracking** — `CostTracker` with `COST_TRACKER` global, `estimate_cost()` for 25+ models across 10 providers, `track_usage()` wired into `complete_via_compat()`, builtins `ai_usage()`, `ai_cost()`, `ai_reset_usage()`
- [x] **Update model metadata** — OpenAI pricing updated for GPT-4o/4.5/o3/o4-mini (Feb 2026), Anthropic expanded to 6 models (Claude 4 Opus/Sonnet, 3.5 Sonnet/Haiku, 3 Opus/Sonnet), embeddings pricing current

#### Local AI Infrastructure
- [x] **Implement `LocalProvider`** — full implementation with model directory scanning, llama.cpp delegation for chat/embeddings, GGUF/SafeTensors/ONNX format detection, local model metadata extraction
- [x] **Ollama model auto-pull** — `OllamaProvider::pull_model()` and `has_model()` methods for on-demand model fetching
- [x] **Backend health monitoring** — Ollama and LM Studio detection in `detect_backends()`, health checks via `validate_api_key()`
- [x] **GPU memory management** — `query_gpu_memory()` via nvidia-smi, `estimate_model_vram_mb()` with quantization-aware sizing (Q4/Q5/Q8/FP16/FP32), `model_fits_gpu()` assessment, builtins `ai_gpu_memory()` and `ai_model_fits()`
- [x] **Model format conversion** — wired `ai_api/converters.rs` to builtins: `ai_convert_model()`, `ai_supported_conversions()`, `ai_detect_format()` for GGUF ↔ SafeTensors ↔ ONNX ↔ PyTorch ↔ TensorFlow conversion

#### Ontology & Discoverability
- [x] **Dynamic builtin discovery** — `get_all_builtin_definitions()` auto-generates schemas from `BUILTIN_LOOKUP` for all 1,100+ builtins with categories, descriptions, return types, and alias detection
- [x] **Connect OS ontology to Agent API** — wired `OS_ONTOLOGY` and `CLI_TOOL_REGISTRY` into `build_language_ontology()` with `os_ontology` and `cli_tools` fields; all 20+ schema builders use unified `ontology_tools()` helper
- [x] **Unify tool schema systems** — added `ToolFormat` enum, `ToolSchema::from_builtin()`, `builtins_to_tools()` bridge in `providers/schema.rs`; all schema builders refactored to use single source of truth, eliminating ~400 lines of duplicated format logic
- [x] **CLI tool ontology mappings** — `CLIToolRegistry` with 90+ tool definitions across 15 categories, `detect_tool()` using `which` crate, `get_tool_version()`, `to_json_schema()` export; `CLI_TOOL_REGISTRY` lazy_static wired into Agent API
- [x] **Module-level schemas** — `get_module_definitions()` exposes typed signatures for all 108 modules with function mappings; `build_compact_ontology()` includes module data
- [x] **Runtime type introspection** — builtins `typeof()`, `type_name()`, `type_fields()`, `type_schema()`, `is_type()` expose runtime Value types; `type_schema()` generates JSON Schema from any Value; `type_name()` provides detailed inner types (e.g., `Array<Int>`, `Record{a: Int, b: String}`)
- [x] **Ontology versioning** — `ontology_version` field on `LanguageOntology`, included in compact and full schema exports
- [x] **Ontology export formats** — added JSON-LD (`to_json_ld()`), OWL/RDF Turtle (`to_owl_turtle()`), and SHACL (`to_shacl()`) exports on `OSOperationRegistry`; Agent API schema endpoint supports `jsonld`, `owl`, `shacl` format params; builtins `ontology_json()`, `ontology_jsonld()`, `ontology_owl()`, `ontology_shacl()`
- [x] **Update AI discoverability files** — synced `llms.txt`, `llms-full.txt`, `AGENTS.md`, `copilot-instructions.md` with LM Studio provider, ontology modules, and schema endpoints

---

## Known Issues

### Resolved
- [x] Parser: pipeline assignments without parentheses
- [x] Parser: proper semicolon statement separation
- [x] Parser: newline-aware word-call parsing
- [x] Parser: zero-parameter lambdas
- [x] Builtins: consistent call syntax (pipeline vs function)
- [x] All 88 compiler warnings fixed

### Open
- [ ] VS Code marketplace PAT expired (publisher `admercs`). Not rechecked; the completion plan found `VSCE_PAT` set on 2026-10-28.
- [ ] Pre-existing test failure: `security_user_simulation::friendly_user_allowed_commands` (validate_command assertion). Not rerun for this edit; the test still exists in `tests/security_user_simulation.rs`.

### Recently Resolved (v1.2.0)
- [x] Flaky `cfg_feature_enabled` test — fixed with Mutex-based env var serialization
- [x] 9 LOW-severity builtins converted to structured output
- [x] Three parallel provider systems unified — `providers/` concrete impls, bridge.rs, routing engine
- [x] `bridge.rs` and `impls/` aligned with `LLMProvider` API (all 180 errors resolved)
- [x] Provider routing rules wired to live completions via `complete_via_registry()`
- [x] `platform` module triple-registration fixed

---

## Metrics

### Test Coverage
- **Test functions (2026-11-20):** 6,701 `#[test]` attributes, counted with
  `grep -rn "#\[test\]" tests src --include=*.rs | wc -l`: 5,683 under `tests/` and
  1,018 under `src/`. This counts declarations only. It misses other test attributes
  (for example `#[tokio::test]`), counts any `#[test]` in a comment, and says nothing
  about how many pass. `#[ignore` appears 141 times in the same two folders.
- **Test files:** 601 `.rs` files directly under `tests/` (2026-11-20)
- **Pass counts:** none claimed here. The last pass counts in this file were from
  2026-10-23 and are removed. Run `cargo test --workspace` for a current result.

### Codebase
- **Language:** Rust
- **Source and test line counts:** not recounted for 2026-11-20; the earlier figures were removed
- **Builtins:** 1,200+ in the ontology, 1,400+ callable names counting aliases (as AGENTS.md states; `tests/documented_builtin_count.rs` pins the counts)
- **Modules:** 108
- **Themes:** 38

### Distribution
Checked against the registries on 2026-11-20 unless noted.

- **crates.io:** [iron-shell 14.0.5](https://crates.io/crates/iron-shell) and [iron-shell-lsp 14.0.5](https://crates.io/crates/iron-shell-lsp). The binary is `ish`; `ae` is an alias.
- **PyPI:** `ironshell` 1.5.2 (the Python SDK; `integrations/python/pyproject.toml` also says 1.5.2)
- **VS Code:** extension `admercs.ironshell`, source version 1.6.1 in `editors/vscode`. The Marketplace listing was not checked.
- **GitHub:** [nervosys/IronShell](https://github.com/nervosys/IronShell), release v14.0.5 (Latest)
- **Homebrew:** `brew tap nervosys/ironshell https://github.com/nervosys/IronShell`, then `brew install nervosys/ironshell/ironshell`. `Formula/ironshell.rb` names the v14.0.5 tarball, and its sha256 matches the tarball GitHub serves for that tag.
- **npm:** nothing has ever been published. `@nervosys/aethershell` returns 404, and no `iron-shell` npm package is claimed.

---

## Carried over from the completion and productization plans

Written 2026-11-20. These are the boxes that stayed open in
[docs/COMPLETION_PLAN.md](docs/COMPLETION_PLAN.md) and
[docs/PRODUCTIZATION_PLAN.md](docs/PRODUCTIZATION_PLAN.md) after each was checked
against CHANGELOG.md, the source and the registries. Items are merged where both
plans listed them. Each is open because no evidence of completion was found, not
because it was shown to be undone.

### Builtin contracts and implementations
- [ ] Classify the roughly 123 `bi_*` functions with explicit unsupported branches into always-missing, platform-conditional and policy constraints (completion plan, section 1).
- [ ] Web, clipboard, UI and input operations: verify on a desktop host. The plan found `gui_*` and `clipboard_*` desktop paths unverified.
- [ ] Package managers: `dnf`/`yum` and `brew` adapters are tested through their parsers only, not end to end on a host that uses them.
- [ ] Verify each operation's positive result against real effects in owned fixtures or disposable environments, not only malformed-call refusals.
- [ ] Finish the remaining unsupported branches, by module. Source shows `job_submit` refuses with E_UNIMPLEMENTED (no job executor). `sso_auth` cannot verify a callback and mints no session. ROADMAP's own entries list `finetune_start`, the `cloud_*` builtins, `workspace_sync` and `repl_broadcast` as refusing the same way; those were not rechecked.
- [ ] Declaration quality: read each body before declaring it; reject surplus, mistyped and conflicting input before any effect; give every declaration examples that exercise optional parameters both ways; check return shape and refusal code on every applicable platform.
- [ ] Effectful contracts: finish containment, audit identity, effect classification, budget and transaction checks, and refuse transaction use where rollback cannot restore the operation.
- [ ] Reach full coverage of implementations and callable names, then remove the interim undeclared-builtin assertions.
- [ ] Keep coded refusals distinct (unsupported, absent tool, failed tool, bad input, policy). `tests/uncoded_error_ratchet.rs` pins a ceiling of remaining uncoded errors.
- [ ] Atomic publication, temporary-file cleanup and rollback for all supported filesystem changes: keep extending the tranche tests.

### Security and reliability follow-ups
- [ ] Recheck agent policy enforcement, effect provenance, aliases, audit protection and network restrictions once the catalogue is final. Reads of the audit key file are still allowed anywhere in agent mode.
- [ ] Close the remaining findings of the 2026-10-30 rename audit (security, then discoverability, usability and performance).
- [ ] CI gaps left for decision: the `#[ignore]` disposable-only tests never run in CI; tests that skip when `tmux`, `openssl`, `git`, `kubectl` or `sqlite3` is absent report green; no memory cap on the test run; the MSRV job only checks; `release.yml` builds tag binaries with no test run; branches without a PR are not tested.

### Build, platforms and release checks
- [ ] MSRV: the manifest now says `rust-version = "1.89"`; the plan validated 1.88. Recheck the MSRV and the `native,candle` and `native,onnx` feature sets against the current tree.
- [ ] Hardware checks (`hw_battery`, `hw_gpu`, `hw_sensors`, `sys_cpu_freq`) on hardware with a battery, GPU and sensors; `hw_sensors` was null on the maintainer's host. GUI, clipboard and service contracts on a desktop host.
- [ ] Rerun `scripts/test_release_version.py` and `scripts/test-release-preflight.ps1` on a release commit under PowerShell 5.1 and 7.
- [ ] Update install, upgrade and troubleshooting instructions and verify each channel's package name and version. Not checked: the VS Code Marketplace listing under the new name, the PyPI extras.
- [ ] Download the release assets and test a clean install on Windows, Linux and macOS (each architecture and channel); smoke-test `ish --version`, typed pipelines, the Agent API, SDK and LSP, upgrade and uninstall; verify published checksums and documentation links.
- [ ] Final pass: every number in the docs matches the command that produced it.
- [ ] Decide the global git `user.email` (maintainer); push the amended `infra` commit if it still exists (not checked).

### Distribution and ecosystem
- [ ] GitHub Linguist submission (needs about 200 repositories with `.ish` files).
- [ ] Chrome Web Store submission of `integrations/browser-extension` (0.2.0). No evidence of a listing.
- [ ] npm package: none exists. Publish one or drop the item.
- [ ] Landing page on the `ironshell.io` domain. `website/` is deployed by `.github/workflows/website.yml`; the domain was not checked.
- [ ] Documentation: a review of the book and API reference for completeness; an introduction video; a launch post.
- [ ] Community: Discord server, a code of conduct (no `CODE_OF_CONDUCT.md` exists), a first community call, a developer-preview announcement, feedback collection, Product Hunt.

### Product features without evidence of completion
- [ ] Interactive debugger (only the `debug` builtin and related tools exist; no interactive debugger was found).
- [ ] Shell history search (fzf-like).
- [ ] Custom plugin system beyond TOML metadata registration; script builtins are refused.
- [ ] Distributed agent execution and workflow persistence (`job_submit` refuses; no workflow persistence builtins were found).
- [ ] Business items: Pro tier, payment integration, license server, enterprise tier launch.
- [ ] SSO (see `sso_auth` above), visual workflow builder, collaborative sessions (`workspace_sync` refuses), mobile companion app.

Two groups of boxes were left unchecked and not carried over. The six "working
checklist for each migration tranche" items in the completion plan are a
recurring procedure, not deliverables. The E7 live-model run is treated as
closed: results exist under `benches/agentic/results/e7-*.md` from Claude
subagent and local-model runs, and no run used an API credential.

---

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines. All contributions require a signed [CLA](CLA.md).

**Priority areas:**
1. **High** — Ecosystem growth (`.ish` scripts, tutorials, blog posts, 200+ repos for Linguist)
2. **Medium** — Expand local inference model support (more architectures, quantization options)
3. **Medium** — VS Code marketplace PAT renewal and extension republish
4. **Good First Issues** — Theme additions, example scripts, documentation improvements

---

## Contact

- **GitHub Issues:** [Report bugs or request features](https://github.com/nervosys/IronShell/issues)
- **Discussions:** [Join the conversation](https://github.com/nervosys/IronShell/discussions)
- **Website:** [nervosys.ai](https://nervosys.ai)
