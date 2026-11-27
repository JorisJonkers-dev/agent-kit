# Changelog

## [6.13.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.12.1...v6.13.0) (2026-11-27)


### Features

* **model-gateway:** run OpenCode's own free models in the sandbox, for public data only ([#987](https://github.com/JorisJonkers-dev/agent-kit/issues/987)) ([50ae424](https://github.com/JorisJonkers-dev/agent-kit/commit/50ae42468b0ac704c1ad9d782a36b899a75c5f36))

## [6.12.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.12.0...v6.12.1) (2026-11-27)


### Bug Fixes

* **model-gateway:** concurrent write runs no longer fail on each other's worktrees, and batch returns the write fields ([#985](https://github.com/JorisJonkers-dev/agent-kit/issues/985)) ([b2750fe](https://github.com/JorisJonkers-dev/agent-kit/commit/b2750fe92a4116e47c89f888eed39f4b1d24bfd1))

## [6.12.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.11.0...v6.12.0) (2026-11-27)


### Features

* **claim:** a cross-device claim command for tickets ([#983](https://github.com/JorisJonkers-dev/agent-kit/issues/983)) ([b7518a6](https://github.com/JorisJonkers-dev/agent-kit/commit/b7518a648c8b7cc8126c58c7794a80ac4a16e2ff))

## [6.11.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.10.1...v6.11.0) (2026-11-27)


### Features

* **model-gateway:** require Docker Sandboxes for every gateway worker and retire confine ([#975](https://github.com/JorisJonkers-dev/agent-kit/issues/975)) ([1a2ee09](https://github.com/JorisJonkers-dev/agent-kit/commit/1a2ee09494e1fbce3212cd22a96a2de7ca0265a6))

## [6.10.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.10.0...v6.10.1) (2026-11-27)


### Bug Fixes

* **setup:** clear dangling uv tool links before install and name sbx's uninitialised network policy ([#977](https://github.com/JorisJonkers-dev/agent-kit/issues/977)) ([6397064](https://github.com/JorisJonkers-dev/agent-kit/commit/639706416ba586bd8b8acc9e1b5c675b5a8bdf84))

## [6.10.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.9.1...v6.10.0) (2026-11-27)


### Features

* **model-gateway:** per-run sandbox network reach and the estate package proxy ([#969](https://github.com/JorisJonkers-dev/agent-kit/issues/969)) ([44a5404](https://github.com/JorisJonkers-dev/agent-kit/commit/44a54044dc6eaa3364c2842046b6e2ff49c13130))


### Bug Fixes

* **plugins:** update anthropics/claude-plugins-official digest to 0fbfda5 ([#887](https://github.com/JorisJonkers-dev/agent-kit/issues/887)) ([2fabbc8](https://github.com/JorisJonkers-dev/agent-kit/commit/2fabbc84c3a70faf8982d0fc41fca136ee54f81d))
* **plugins:** update jgraph/drawio-mcp digest to b7941af ([#926](https://github.com/JorisJonkers-dev/agent-kit/issues/926)) ([bb3c7ec](https://github.com/JorisJonkers-dev/agent-kit/commit/bb3c7eca9b968ebc6ac2be0e6132a28be4a4b638))

## [6.9.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.9.0...v6.9.1) (2026-11-27)


### Bug Fixes

* **setup:** read the packages proxy key from the cluster; skip non-UTF-8 environment entries ([#972](https://github.com/JorisJonkers-dev/agent-kit/issues/972)) ([75d05fd](https://github.com/JorisJonkers-dev/agent-kit/commit/75d05fd8f2b3dc891369a769f0c067c66b5b2881))

## [6.9.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.8.0...v6.9.0) (2026-11-27)


### Features

* **model-gateway:** run Claude Code, Codex and Hermes in Docker Sandboxes under AGENT_KIT_SANDBOX=sbx ([#971](https://github.com/JorisJonkers-dev/agent-kit/issues/971)) ([1a5d230](https://github.com/JorisJonkers-dev/agent-kit/commit/1a5d230a00c39fff8e75e74641c1ee40f804f445))

## [6.8.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.7.0...v6.8.0) (2026-11-27)


### Features

* **model-gateway:** run read-only Vibe launches in Docker Sandboxes under AGENT_KIT_SANDBOX=sbx ([#968](https://github.com/JorisJonkers-dev/agent-kit/issues/968)) ([8366a99](https://github.com/JorisJonkers-dev/agent-kit/commit/8366a996f00ac318ca09caf8d3183706980694b3))
* **model-gateway:** run writing Pi and OpenCode launches in an sbx sandbox and prune orphaned sandboxes ([#967](https://github.com/JorisJonkers-dev/agent-kit/issues/967)) ([d3976ae](https://github.com/JorisJonkers-dev/agent-kit/commit/d3976ae6928652674aad7da4d257b48b5017177b))

## [6.7.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.6.0...v6.7.0) (2026-11-27)


### Features

* **keys:** a dedicated paid key, and the go-cheap and vibe tiers reach paid/&lt;model&gt; ([#966](https://github.com/JorisJonkers-dev/agent-kit/issues/966)) ([22a0714](https://github.com/JorisJonkers-dev/agent-kit/commit/22a0714199d7232fa848ea6f2d625d642138fd0d)), closes [#965](https://github.com/JorisJonkers-dev/agent-kit/issues/965)
* **model-gateway:** plan each launch for Docker Sandboxes behind AGENT_KIT_SANDBOX=sbx-dry-run ([#951](https://github.com/JorisJonkers-dev/agent-kit/issues/951)) ([8707cda](https://github.com/JorisJonkers-dev/agent-kit/commit/8707cda1b505ff534bb064ae1e211d107f3d1442))
* **model-gateway:** run read-only Pi and OpenCode launches in Docker Sandboxes behind AGENT_KIT_SANDBOX=sbx ([#954](https://github.com/JorisJonkers-dev/agent-kit/issues/954)) ([d6c1a92](https://github.com/JorisJonkers-dev/agent-kit/commit/d6c1a926d709d1b8ced28af007ab1007dfb06c0c))
* **model-gateway:** run sbx sandboxes from Docker's prebuilt images pinned by digest ([#955](https://github.com/JorisJonkers-dev/agent-kit/issues/955)) ([e13702f](https://github.com/JorisJonkers-dev/agent-kit/commit/e13702f9f8349533b780b63f79cf06d6c86533c2))
* **sbx:** pinned pi and OpenCode sandbox images and their build script ([#953](https://github.com/JorisJonkers-dev/agent-kit/issues/953)) ([b322b56](https://github.com/JorisJonkers-dev/agent-kit/commit/b322b569c8d39dab459ac346a477fd764c3e916a))
* **sbx:** set up Docker Sandboxes and pre-pull gateway images in workstation setup ([#956](https://github.com/JorisJonkers-dev/agent-kit/issues/956)) ([f8d3d70](https://github.com/JorisJonkers-dev/agent-kit/commit/f8d3d70d8489d90c349b9946c893e71c752b5957))

## [6.6.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.5.0...v6.6.0) (2026-11-26)


### Features

* **critique:** default every review route to Flash-class, small or free models ([#946](https://github.com/JorisJonkers-dev/agent-kit/issues/946)) ([9c768ed](https://github.com/JorisJonkers-dev/agent-kit/commit/9c768edfdaadfc716e8b18f4cc3493a7b0fc918f))
* **model-gateway:** decide the family allowlist by repository visibility ([#931](https://github.com/JorisJonkers-dev/agent-kit/issues/931)) ([772e512](https://github.com/JorisJonkers-dev/agent-kit/commit/772e5127d20816c3ac14f6f16c15979a15024064))
* **model-gateway:** reach free models only as free/&lt;model&gt; on the free key ([#947](https://github.com/JorisJonkers-dev/agent-kit/issues/947)) ([c7ccfb2](https://github.com/JorisJonkers-dev/agent-kit/commit/c7ccfb21e2872f47ee9e9d7d6bcec94d439b5a93))
* **model-gateway:** stream each run into an event log and say why it failed ([#948](https://github.com/JorisJonkers-dev/agent-kit/issues/948)) ([ad19ca0](https://github.com/JorisJonkers-dev/agent-kit/commit/ad19ca0e573bce3a4869465b76b8243a78c22742))
* **registry:** refresh the LiteLLM catalogue after the first full free-route sync ([#928](https://github.com/JorisJonkers-dev/agent-kit/issues/928)) ([6f62bab](https://github.com/JorisJonkers-dev/agent-kit/commit/6f62bab5a9431c878a3a7266a1483644dc67c1a5))


### Bug Fixes

* **model-gateway:** let a confined Vibe read the project configs above its working directory ([#949](https://github.com/JorisJonkers-dev/agent-kit/issues/949)) ([d83679e](https://github.com/JorisJonkers-dev/agent-kit/commit/d83679e840eada5a89fd54a1d42333f9084e16d9))
* **model-gateway:** let every confined client see the git of its worktree ([#933](https://github.com/JorisJonkers-dev/agent-kit/issues/933)) ([6c084eb](https://github.com/JorisJonkers-dev/agent-kit/commit/6c084eb68dd9a1973f39389b8d27aa0a8681817c))
* **setup:** leave Claude Code's built-in plugins alone when converging ([#932](https://github.com/JorisJonkers-dev/agent-kit/issues/932)) ([8c059eb](https://github.com/JorisJonkers-dev/agent-kit/commit/8c059eb3ffbdb8e3d3e72d90ca3b12bf0f73a9f3))

## [6.5.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.4.0...v6.5.0) (2026-11-18)


### Features

* **agent-kit-merge:** converge MCP servers and Claude plugins in Rust ([#810](https://github.com/JorisJonkers-dev/agent-kit/issues/810)) ([#874](https://github.com/JorisJonkers-dev/agent-kit/issues/874)) ([15d2d93](https://github.com/JorisJonkers-dev/agent-kit/commit/15d2d931b80beec06cf4357cb87cbea5753ae120))
* **agent-kit-merge:** converge the Gold Band role profiles in Rust ([#810](https://github.com/JorisJonkers-dev/agent-kit/issues/810)) ([#884](https://github.com/JorisJonkers-dev/agent-kit/issues/884)) ([12c78e6](https://github.com/JorisJonkers-dev/agent-kit/commit/12c78e603c2d9caa912bc02def85400e3a259c6e))
* **agent-kit-merge:** converge the Gold Band workflow templates in Rust ([#810](https://github.com/JorisJonkers-dev/agent-kit/issues/810)) ([#885](https://github.com/JorisJonkers-dev/agent-kit/issues/885)) ([9d571f3](https://github.com/JorisJonkers-dev/agent-kit/commit/9d571f3d64f4d49e5f03bc4a0a1152bb57d3d34f))
* **agent-launcher:** hold the confinement on macOS with a deny-default Seatbelt profile ([#917](https://github.com/JorisJonkers-dev/agent-kit/issues/917)) ([6983b89](https://github.com/JorisJonkers-dev/agent-kit/commit/6983b89c5ccb589dfbf113dabf5525b3d0b3e220))
* **bootstrap:** install the release binaries with a short shell, and package them for NixOS ([#807](https://github.com/JorisJonkers-dev/agent-kit/issues/807)) ([#836](https://github.com/JorisJonkers-dev/agent-kit/issues/836)) ([cec2cc1](https://github.com/JorisJonkers-dev/agent-kit/commit/cec2cc142a50e38286644fd37d60566038f84c50))
* **claude:** permissive auto mode defaults for every Claude profile ([#889](https://github.com/JorisJonkers-dev/agent-kit/issues/889)) ([db02b6d](https://github.com/JorisJonkers-dev/agent-kit/commit/db02b6da45a1f3c2b12087d358d59afee3975354))
* **critique:** a free-only critic roster for public repositories ([#854](https://github.com/JorisJonkers-dev/agent-kit/issues/854)) ([#873](https://github.com/JorisJonkers-dev/agent-kit/issues/873)) ([64b2f68](https://github.com/JorisJonkers-dev/agent-kit/commit/64b2f68897b080d6bd4b6b0577464402d2ca8462))
* **critique:** higher critic and profile caps, a larger Pi output cap, and more OpenCode Go flash models ([#879](https://github.com/JorisJonkers-dev/agent-kit/issues/879)) ([a11feea](https://github.com/JorisJonkers-dev/agent-kit/commit/a11feea7ce0049866d851a1a65afc6886c03968c))
* **critique:** run spec critique on tier-review from one command ([#924](https://github.com/JorisJonkers-dev/agent-kit/issues/924)) ([9acf92e](https://github.com/JorisJonkers-dev/agent-kit/commit/9acf92ea258a8910ae3eb598e155bedb93153a0c))
* **estate:** port the estate skill scripts to the estate crate ([#813](https://github.com/JorisJonkers-dev/agent-kit/issues/813)) ([#839](https://github.com/JorisJonkers-dev/agent-kit/issues/839)) ([00b0bbd](https://github.com/JorisJonkers-dev/agent-kit/commit/00b0bbd145efbfa01766b9f2dd0c5cb8f0e0f14a))
* **gold-band-run-start:** resolve the family allowlist from the bound model ([#783](https://github.com/JorisJonkers-dev/agent-kit/issues/783)) ([#829](https://github.com/JorisJonkers-dev/agent-kit/issues/829)) ([49a8ad1](https://github.com/JorisJonkers-dev/agent-kit/commit/49a8ad187573f6c902f8eac65752887bfd11a0eb))
* **gold-band:** workflow-b v2 with TDD slices, a test critic, gate repair edges and a two-axis code review ([#835](https://github.com/JorisJonkers-dev/agent-kit/issues/835)) ([0c0691f](https://github.com/JorisJonkers-dev/agent-kit/commit/0c0691f1a6968dea2b509d2ea66e639812ae0b9f))
* **litellm:** a free route and key for the free access group ([#848](https://github.com/JorisJonkers-dev/agent-kit/issues/848)) ([#859](https://github.com/JorisJonkers-dev/agent-kit/issues/859)) ([00198b7](https://github.com/JorisJonkers-dev/agent-kit/commit/00198b79096aae2ea94303c348f35b01664665d9))
* **litellm:** make-admin-token.sh asks for the master key on a terminal ([#902](https://github.com/JorisJonkers-dev/agent-kit/issues/902)) ([edf7d47](https://github.com/JorisJonkers-dev/agent-kit/commit/edf7d4744b85cb6c2713ef00c83e89d1af0fe7b8))
* **merge:** port the merge, block and proxy scripts to a Rust config-and-merge crate ([#844](https://github.com/JorisJonkers-dev/agent-kit/issues/844)) ([7443d4d](https://github.com/JorisJonkers-dev/agent-kit/commit/7443d4dd853603e38f1a89cebbeddfc426f1cd9a))
* **model-gateway:** a catalog view of every canonical model ([#851](https://github.com/JorisJonkers-dev/agent-kit/issues/851)) ([#863](https://github.com/JorisJonkers-dev/agent-kit/issues/863)) ([59b9cc5](https://github.com/JorisJonkers-dev/agent-kit/commit/59b9cc58b2a1c93b1bc280cea378d2443aeb6b9e))
* **model-gateway:** a harness matrix that proves every role on every harness ([#856](https://github.com/JorisJonkers-dev/agent-kit/issues/856)) ([#869](https://github.com/JorisJonkers-dev/agent-kit/issues/869)) ([bf5bbed](https://github.com/JorisJonkers-dev/agent-kit/commit/bf5bbedb6fb7809f2972185de4d9bba6025dbf35))
* **model-gateway:** catalog shows each deployment's budget pool, its cap and spend ([#851](https://github.com/JorisJonkers-dev/agent-kit/issues/851)) ([#872](https://github.com/JorisJonkers-dev/agent-kit/issues/872)) ([c2296a2](https://github.com/JorisJonkers-dev/agent-kit/commit/c2296a21441cb9fe6ba697e77afd78f05231ee20))
* **model-gateway:** choose the tier by remaining quota ([#923](https://github.com/JorisJonkers-dev/agent-kit/issues/923)) ([a5554cb](https://github.com/JorisJonkers-dev/agent-kit/commit/a5554cb5bc99ab2d35db85028744e80c6211bffe))
* **model-gateway:** delegation roles as profiles, and run --profile --harness ([#855](https://github.com/JorisJonkers-dev/agent-kit/issues/855)) ([#867](https://github.com/JorisJonkers-dev/agent-kit/issues/867)) ([71c5f6e](https://github.com/JorisJonkers-dev/agent-kit/commit/71c5f6e1ebad0ebdaf5431c065661f396a1a58e9))
* **model-gateway:** gated pi-openrouter and pi-free clients, free models only ([#861](https://github.com/JorisJonkers-dev/agent-kit/issues/861)) ([1a53a6c](https://github.com/JorisJonkers-dev/agent-kit/commit/1a53a6c2f464dc882ace592ab5e9b5f185ee2757))
* **model-gateway:** hermes runs a profile's Pi-route models on its own LiteLLM providers ([#878](https://github.com/JorisJonkers-dev/agent-kit/issues/878)) ([5766531](https://github.com/JorisJonkers-dev/agent-kit/commit/5766531ed457b09d3eaaf63801a2ff7d78707c96))
* **model-gateway:** models carries each model's cost and the providers that serve it ([#850](https://github.com/JorisJonkers-dev/agent-kit/issues/850)) ([#862](https://github.com/JorisJonkers-dev/agent-kit/issues/862)) ([b20543f](https://github.com/JorisJonkers-dev/agent-kit/commit/b20543f0fa372887a4f988aaf01ae03d7f0fa5c3))
* **model-gateway:** one free definition, from the catalogue's free flag ([#860](https://github.com/JorisJonkers-dev/agent-kit/issues/860)) ([f8d92be](https://github.com/JorisJonkers-dev/agent-kit/commit/f8d92be42cf68f0e6613611019eecffe3fcb22bd))
* **model-gateway:** OpenCode's own free models, and every role on both Pi and OpenCode ([#898](https://github.com/JorisJonkers-dev/agent-kit/issues/898)) ([68d6f2c](https://github.com/JorisJonkers-dev/agent-kit/commit/68d6f2cf1615c00cc0270115ebdc71d73fc1bd26))
* **model-gateway:** place every usable catalogue model, not only ones with a known family ([#849](https://github.com/JorisJonkers-dev/agent-kit/issues/849)) ([#858](https://github.com/JorisJonkers-dev/agent-kit/issues/858)) ([1871674](https://github.com/JorisJonkers-dev/agent-kit/commit/18716744b4b4b1a8562ec828d22d50538a82e14c))
* **model-gateway:** the Vibe CLI runs only when a caller names it, never from a profile or a critic fallback ([#882](https://github.com/JorisJonkers-dev/agent-kit/issues/882)) ([e20a4a8](https://github.com/JorisJonkers-dev/agent-kit/commit/e20a4a8d9c81c457314ea8b749108cb736aac674))
* **model-gateway:** tier profiles from litellm, --tier, and a routing record per run ([#920](https://github.com/JorisJonkers-dev/agent-kit/issues/920)) ([458f9d5](https://github.com/JorisJonkers-dev/agent-kit/commit/458f9d5608ce1a95720f6895e9997d78751e981e))
* **pr-composer:** port check_body and mine_prs to the pr-composer Rust crate ([#840](https://github.com/JorisJonkers-dev/agent-kit/issues/840)) ([8a92525](https://github.com/JorisJonkers-dev/agent-kit/commit/8a925255e8c4cc36628ffa977f39ec3101cb0730))
* **projects-layout:** port the projects-layout guard hook to Rust ([#838](https://github.com/JorisJonkers-dev/agent-kit/issues/838)) ([1e87553](https://github.com/JorisJonkers-dev/agent-kit/commit/1e8755359957b5ad669b813bb074f489a9f70aac))
* **registry:** render the new free routes and give their models tiers ([#918](https://github.com/JorisJonkers-dev/agent-kit/issues/918)) ([2709946](https://github.com/JorisJonkers-dev/agent-kit/commit/2709946cc1b9bf0661da01c3ced101eb4e282787))
* **rust:** port the secrets, kube and LiteLLM scripts to Rust ([#812](https://github.com/JorisJonkers-dev/agent-kit/issues/812)) ([#843](https://github.com/JorisJonkers-dev/agent-kit/issues/843)) ([cf52df9](https://github.com/JorisJonkers-dev/agent-kit/commit/cf52df94d0d34d39de213c1d3ecdeefb5ee3a9e4))
* skills and Gold Band workflows choose models by tier ([#922](https://github.com/JorisJonkers-dev/agent-kit/issues/922)) ([6b14a3f](https://github.com/JorisJonkers-dev/agent-kit/commit/6b14a3fe702d4fa9e99a97775cb938d58ec4a032))


### Bug Fixes

* **agent-kit-keys:** mint again a stored LiteLLM key scoped narrower than its team ([#877](https://github.com/JorisJonkers-dev/agent-kit/issues/877)) ([40e7c42](https://github.com/JorisJonkers-dev/agent-kit/commit/40e7c42244edc3f2f9b394fa15bb2b5a09c45419))
* **claude:** drop the Write deny rules from the personal profile overlay ([#890](https://github.com/JorisJonkers-dev/agent-kit/issues/890)) ([db76433](https://github.com/JorisJonkers-dev/agent-kit/commit/db764334c2f8eca9ce56d2174f5d47e5f8e1c274))
* **critique:** read a change that touches a submodule instead of failing triage ([#833](https://github.com/JorisJonkers-dev/agent-kit/issues/833)) ([50dc812](https://github.com/JorisJonkers-dev/agent-kit/commit/50dc81268ee7690ee023170d154c126c5b6fa2f9))
* **delegate:** allow pi-openrouter and pi-free for public-data work ([#866](https://github.com/JorisJonkers-dev/agent-kit/issues/866)) ([c88a2ca](https://github.com/JorisJonkers-dev/agent-kit/commit/c88a2ca4c5a73f7647088a35d46659517d96d560))
* **deps:** update minor and patch dependencies ([#846](https://github.com/JorisJonkers-dev/agent-kit/issues/846)) ([67a4e78](https://github.com/JorisJonkers-dev/agent-kit/commit/67a4e785ddf74d9c5dcb588d3c180b354f7870f3))
* **model-gateway:** a confined Pi launch keeps PI_CODING_AGENT_DIR and TMPDIR ([#870](https://github.com/JorisJonkers-dev/agent-kit/issues/870)) ([#871](https://github.com/JorisJonkers-dev/agent-kit/issues/871)) ([b544291](https://github.com/JorisJonkers-dev/agent-kit/commit/b54429124f92f0dbdf5ce4e4aab515447a81b01e))
* **model-gateway:** a read-only agy run may read files and is told it cannot ask for permission ([#876](https://github.com/JorisJonkers-dev/agent-kit/issues/876)) ([6948215](https://github.com/JorisJonkers-dev/agent-kit/commit/69482157996e4c8b6586705fec0f6130a4bd402e))
* **model-gateway:** give each delegate a per-run home, read-only tools and no home MCP ([#919](https://github.com/JorisJonkers-dev/agent-kit/issues/919)) ([290296f](https://github.com/JorisJonkers-dev/agent-kit/commit/290296f12892f7ceb4812c638a006cf8f347722d))
* **model-gateway:** let public-only routes read repositories recorded as public ([#892](https://github.com/JorisJonkers-dev/agent-kit/issues/892)) ([681aeef](https://github.com/JorisJonkers-dev/agent-kit/commit/681aeef7cd071630ceebb80a2c1d0ccf5bde01be))
* **model-gateway:** models runs with no arguments, and the integration doc matches SKILL.md ([#852](https://github.com/JorisJonkers-dev/agent-kit/issues/852)) ([#864](https://github.com/JorisJonkers-dev/agent-kit/issues/864)) ([e980244](https://github.com/JorisJonkers-dev/agent-kit/commit/e9802440921308e92aa0ac75afba807f48f3283f))
* **model-gateway:** say why a Pi route has no provider, rename the public-free profile, refresh the catalogue snapshot ([#853](https://github.com/JorisJonkers-dev/agent-kit/issues/853)) ([#865](https://github.com/JorisJonkers-dev/agent-kit/issues/865)) ([8060017](https://github.com/JorisJonkers-dev/agent-kit/commit/806001782e751749cb4c09d6cc02d3a0bdb85ab1))
* **model-gateway:** start every client through agent-launcher confine ([#921](https://github.com/JorisJonkers-dev/agent-kit/issues/921)) ([9176639](https://github.com/JorisJonkers-dev/agent-kit/commit/91766393e47f321c37115d9a48e6d6dc3a9095f3))
* **pi:** list only usable models in the Pi provider table ([#837](https://github.com/JorisJonkers-dev/agent-kit/issues/837)) ([3f051d7](https://github.com/JorisJonkers-dev/agent-kit/commit/3f051d787bafd1ba64dc57488db8624877db6702))
* **plugins:** update jgraph/drawio-mcp digest to 87a7a2a ([#845](https://github.com/JorisJonkers-dev/agent-kit/issues/845)) ([d2d0696](https://github.com/JorisJonkers-dev/agent-kit/commit/d2d0696b3f3093a39c036068050a63748e28f374))
* **plugins:** update jgraph/drawio-mcp digest to 9bd19b3 ([#888](https://github.com/JorisJonkers-dev/agent-kit/issues/888)) ([7143d0b](https://github.com/JorisJonkers-dev/agent-kit/commit/7143d0b87482c4d8fa2b8d23efa9bc4b5ba1cc9f))
* **projects-layout-guard:** print usage on --help so setup's smoke check sees it start ([#875](https://github.com/JorisJonkers-dev/agent-kit/issues/875)) ([8c723ac](https://github.com/JorisJonkers-dev/agent-kit/commit/8c723ac3c28fce123eae78bda46caa3b93d30f10))
* **routing:** drop the unusable public gemini model and check rosters against the catalogue snapshot ([#868](https://github.com/JorisJonkers-dev/agent-kit/issues/868)) ([4c04b7c](https://github.com/JorisJonkers-dev/agent-kit/commit/4c04b7c7d11211f40c36c9348286229b03d64857))
* **setup:** register code-nav by path, name the command for each owner step ([#831](https://github.com/JorisJonkers-dev/agent-kit/issues/831)) ([ad35ea1](https://github.com/JorisJonkers-dev/agent-kit/commit/ad35ea1b306299474f273b83cf159e187f9849bc))
* **setup:** say the delegate skill plans delegations, not that it is coming later ([#825](https://github.com/JorisJonkers-dev/agent-kit/issues/825)) ([15846cb](https://github.com/JorisJonkers-dev/agent-kit/commit/15846cb61556233c600b5d50c8551f865b69c0f7))

## [6.4.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.3.0...v6.4.0) (2026-11-15)


### Features

* **build-tracker:** port render --tickets to the Rust crate and swap it in ([#817](https://github.com/JorisJonkers-dev/agent-kit/issues/817)) ([1301a89](https://github.com/JorisJonkers-dev/agent-kit/commit/1301a898a97b12d1ac77e06f790078463e89b6cc))
* **gold-band:** generated critique block with accept, revise and escalate for spec and tickets ([#823](https://github.com/JorisJonkers-dev/agent-kit/issues/823)) ([b1e114e](https://github.com/JorisJonkers-dev/agent-kit/commit/b1e114ee9ed3a5cb12abaf3f50487de443bc3f75))
* **litellm:** let the openrouter key reach OpenCode Go's free models ([#824](https://github.com/JorisJonkers-dev/agent-kit/issues/824)) ([217b7ff](https://github.com/JorisJonkers-dev/agent-kit/commit/217b7ff20f8f1984585c7fd7c4b1790d9e4ddc60))
* **model-gateway:** a verified free cost class and free-only runs ([#822](https://github.com/JorisJonkers-dev/agent-kit/issues/822)) ([b542212](https://github.com/JorisJonkers-dev/agent-kit/commit/b542212a398763ea81eaed1c0cb71121ecfba058)), closes [#819](https://github.com/JorisJonkers-dev/agent-kit/issues/819)


### Bug Fixes

* **gold-band:** keep a critic of the writer's family out of workflow-c reviews ([#818](https://github.com/JorisJonkers-dev/agent-kit/issues/818)) ([e8344ac](https://github.com/JorisJonkers-dev/agent-kit/commit/e8344ac6061b72859971b82d770552ed5316f76a))
* **litellm:** mark a model free only when every deployment is free ([#820](https://github.com/JorisJonkers-dev/agent-kit/issues/820)) ([3f69fd3](https://github.com/JorisJonkers-dev/agent-kit/commit/3f69fd3af11468aa311ed17fbb3296226e751689)), closes [#819](https://github.com/JorisJonkers-dev/agent-kit/issues/819)

## [6.3.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.2.1...v6.3.0) (2026-11-15)


### Features

* **model-gateway:** delegation profiles that own the allowed models and the tool scope ([#792](https://github.com/JorisJonkers-dev/agent-kit/issues/792)) ([2d534ab](https://github.com/JorisJonkers-dev/agent-kit/commit/2d534abc89d7fd9ab02c1f48a8c4c1dc552fe0a4)), closes [#774](https://github.com/JorisJonkers-dev/agent-kit/issues/774) [#773](https://github.com/JorisJonkers-dev/agent-kit/issues/773)
* **model-gateway:** enforce a profile's skills and plugins on Pi with a per-run home ([#801](https://github.com/JorisJonkers-dev/agent-kit/issues/801)) ([cf543d9](https://github.com/JorisJonkers-dev/agent-kit/commit/cf543d96460fc9a64ba78f512276517d35ed7130))
* **workflow-lint:** critic-required and critic-family rules for Gold Band templates ([#800](https://github.com/JorisJonkers-dev/agent-kit/issues/800)) ([4a8539f](https://github.com/JorisJonkers-dev/agent-kit/commit/4a8539fa7d53e81ac3d6b6c99c699c3e5c2f54db))
* **workflow-lint:** enforce Gold Band's editor validation rules on shipped templates ([#782](https://github.com/JorisJonkers-dev/agent-kit/issues/782)) ([da78a02](https://github.com/JorisJonkers-dev/agent-kit/commit/da78a02259758b8bdbc9b1927e64c1e35ea6d05d)), closes [#780](https://github.com/JorisJonkers-dev/agent-kit/issues/780)


### Bug Fixes

* **gold-band:** add success expressions and the grant-refused end edge to the shipped templates ([#790](https://github.com/JorisJonkers-dev/agent-kit/issues/790)) ([046aecb](https://github.com/JorisJonkers-dev/agent-kit/commit/046aecb7b668525079bd6764ca20ce137c84a4f7)), closes [#781](https://github.com/JorisJonkers-dev/agent-kit/issues/781)
* **gold-band:** findable desktop entry, NixOS launcher and settingsSchemaVersion ([#766](https://github.com/JorisJonkers-dev/agent-kit/issues/766)) ([76d0eef](https://github.com/JorisJonkers-dev/agent-kit/commit/76d0eeff0783c1cdd1339766a4420278fa3fc5bf))
* **gold-band:** patch the editor entry rule so failure targets are not entries ([#791](https://github.com/JorisJonkers-dev/agent-kit/issues/791)) ([a7d215e](https://github.com/JorisJonkers-dev/agent-kit/commit/a7d215ee77c676edeb0bb10d9d7bd2d8ba0ad961)), closes [#781](https://github.com/JorisJonkers-dev/agent-kit/issues/781)
* **litellm:** withhold Claude models from every workstation route ([#816](https://github.com/JorisJonkers-dev/agent-kit/issues/816)) ([e6e1647](https://github.com/JorisJonkers-dev/agent-kit/commit/e6e1647018d26d7a3aaf96bc77f7dc94e235300d))
* **setup:** install hermes-agent with its acp extra and fail --check on missing Gold Band templates ([#779](https://github.com/JorisJonkers-dev/agent-kit/issues/779)) ([766531d](https://github.com/JorisJonkers-dev/agent-kit/commit/766531d370a598e106f2710564469c844fe3ead5))

## [6.2.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.2.0...v6.2.1) (2026-11-15)


### Bug Fixes

* **model-gateway:** bound JSON nesting, keep lone surrogates and index object keys ([#761](https://github.com/JorisJonkers-dev/agent-kit/issues/761)) ([aa27c51](https://github.com/JorisJonkers-dev/agent-kit/commit/aa27c51defcc6368add54a183c2c2b2b2aa01970))

## [6.2.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.1.0...v6.2.0) (2026-11-14)


### Features

* **agent-launcher:** confine a whole process to a read allowlist with Landlock ([#744](https://github.com/JorisJonkers-dev/agent-kit/issues/744)) ([fdb73bf](https://github.com/JorisJonkers-dev/agent-kit/commit/fdb73bf802d09903a8c1a0e9733b68b9d93d82bb))
* **critique:** add a pi critic client kind for critics and refuters on LiteLLM routes ([#743](https://github.com/JorisJonkers-dev/agent-kit/issues/743)) ([bbe07ba](https://github.com/JorisJonkers-dev/agent-kit/commit/bbe07ba19966ef0ccf5adeae37090e3150c53542))
* **critique:** make Pi over LiteLLM the default critic and keep public-route clients off private repositories ([#760](https://github.com/JorisJonkers-dev/agent-kit/issues/760)) ([d0aa1b6](https://github.com/JorisJonkers-dev/agent-kit/commit/d0aa1b6e1490e9b6e8f7db87b6279457db1c48ed))
* **critique:** run every Pi critic confined to its packet by agent-launcher ([#747](https://github.com/JorisJonkers-dev/agent-kit/issues/747)) ([c44759b](https://github.com/JorisJonkers-dev/agent-kit/commit/c44759b62836467adade3c1007f1e3f1c0fb6b91))
* **critique:** stats report precision per check and the refute round's outcomes ([#754](https://github.com/JorisJonkers-dev/agent-kit/issues/754)) ([762329e](https://github.com/JorisJonkers-dev/agent-kit/commit/762329eeef81d47c2167f9223f3fea4d64a9854b)), closes [#679](https://github.com/JorisJonkers-dev/agent-kit/issues/679)
* **gold-band:** bind Workflow B to Pi and Claude and run it live with pinned shim agents ([#755](https://github.com/JorisJonkers-dev/agent-kit/issues/755)) ([6abc214](https://github.com/JorisJonkers-dev/agent-kit/commit/6abc21483254c2ad516cfb8850510fd741f16667))
* **gold-band:** hold the work instance's Hindsight server to the Nedap bank ([#749](https://github.com/JorisJonkers-dev/agent-kit/issues/749)) ([7371c20](https://github.com/JorisJonkers-dev/agent-kit/commit/7371c20cd8c5b6039aa7e781412b44e2dc05262b))
* **gold-band:** Workflow A, deep research, in the personal instance ([#456](https://github.com/JorisJonkers-dev/agent-kit/issues/456)) ([#746](https://github.com/JorisJonkers-dev/agent-kit/issues/746)) ([54a2b60](https://github.com/JorisJonkers-dev/agent-kit/commit/54a2b60473c9785874f5fcedd67d2afc555453c8))
* **launcher:** confine Pi critics harder and give them the search server ([#762](https://github.com/JorisJonkers-dev/agent-kit/issues/762)) ([97443b6](https://github.com/JorisJonkers-dev/agent-kit/commit/97443b6095715a4e97a8f7f6e2da18545d298f5b))
* **litellm:** make the free OpenRouter route public-only, and keys follow their team's models ([#739](https://github.com/JorisJonkers-dev/agent-kit/issues/739)) ([3dcdf2f](https://github.com/JorisJonkers-dev/agent-kit/commit/3dcdf2fd022d71780b2b3a5ed9f5c4304781f62d))
* **litellm:** Pi provider models come from the live catalogue's canonical groups ([#568](https://github.com/JorisJonkers-dev/agent-kit/issues/568)) ([#750](https://github.com/JorisJonkers-dev/agent-kit/issues/750)) ([7f3f8e0](https://github.com/JorisJonkers-dev/agent-kit/commit/7f3f8e02a53a2b7fe03954d5b163e94eefe769ff))
* **litellm:** run Claude Code, Codex, Hermes, OpenCode and Vibe on a LiteLLM route with its key from the secret store ([#737](https://github.com/JorisJonkers-dev/agent-kit/issues/737)) ([#757](https://github.com/JorisJonkers-dev/agent-kit/issues/757)) ([2ff3d53](https://github.com/JorisJonkers-dev/agent-kit/commit/2ff3d537ec1e870cb2a7e515e0c6bb74ff8add61))
* **litellm:** setup mints each host's agent keys into per-route teams ([#734](https://github.com/JorisJonkers-dev/agent-kit/issues/734)) ([3fc6f12](https://github.com/JorisJonkers-dev/agent-kit/commit/3fc6f122d2138a2a5566f2b64e7cffd61e2ddbb8))
* **litellm:** the local key also holds local/*, so local/&lt;id&gt; answers ([#745](https://github.com/JorisJonkers-dev/agent-kit/issues/745)) ([5de7b08](https://github.com/JorisJonkers-dev/agent-kit/commit/5de7b082f83ef08e31a2b1c82b24cf4edafd62cd)), closes [#569](https://github.com/JorisJonkers-dev/agent-kit/issues/569)
* **model-gateway:** one Pi client per LiteLLM route, started through agent-launcher confine ([#737](https://github.com/JorisJonkers-dev/agent-kit/issues/737)) ([#763](https://github.com/JorisJonkers-dev/agent-kit/issues/763)) ([c1cccf2](https://github.com/JorisJonkers-dev/agent-kit/commit/c1cccf217c60641d66c10b88a1c3edf2b1ba036a))
* **model-gateway:** read the live LiteLLM catalogue for usable, blocked and free models ([#740](https://github.com/JorisJonkers-dev/agent-kit/issues/740)) ([b03c591](https://github.com/JorisJonkers-dev/agent-kit/commit/b03c591b86f341793f3a1f992e0ea6b6f58186c0))
* **pi:** setup writes one Pi provider per LiteLLM route into models.json ([#738](https://github.com/JorisJonkers-dev/agent-kit/issues/738)) ([7b90249](https://github.com/JorisJonkers-dev/agent-kit/commit/7b90249e673b8f3b75036e91c9c985f8a28e704f))
* **rust:** retire the crate gate for the packages proxy and cargo's own 7-day cooldown ([#733](https://github.com/JorisJonkers-dev/agent-kit/issues/733)) ([7691eed](https://github.com/JorisJonkers-dev/agent-kit/commit/7691eed7a4c1bdadaaf999f09461026f682b2c7f))
* **setup:** build the kit's command-line tools from the pinned workspace into ~/.local/bin ([#759](https://github.com/JorisJonkers-dev/agent-kit/issues/759)) ([2c80375](https://github.com/JorisJonkers-dev/agent-kit/commit/2c803751d434c138bb613ebcd763eff566057d51))
* **setup:** converge the kit's workflow templates into the personal Gold Band instance ([#758](https://github.com/JorisJonkers-dev/agent-kit/issues/758)) ([12d372d](https://github.com/JorisJonkers-dev/agent-kit/commit/12d372de81cc59cec6506b00be7e02e689e6e479))


### Bug Fixes

* **critique:** the gates judge commands whose directory is unclear and find git when PATH lacks it ([#753](https://github.com/JorisJonkers-dev/agent-kit/issues/753)) ([d7a05c4](https://github.com/JorisJonkers-dev/agent-kit/commit/d7a05c46dd06c1d35f80ca174c01c1af1ff8a4a9)), closes [#678](https://github.com/JorisJonkers-dev/agent-kit/issues/678)
* **private:** turn echo off before the passphrase prompt is shown ([#748](https://github.com/JorisJonkers-dev/agent-kit/issues/748)) ([0da8add](https://github.com/JorisJonkers-dev/agent-kit/commit/0da8addc67225afaf01292c014c9e1d365720aa8))
* **setup:** fold a secondary Claude profile's own settings.json back into the shared one ([#730](https://github.com/JorisJonkers-dev/agent-kit/issues/730)) ([d01a74d](https://github.com/JorisJonkers-dev/agent-kit/commit/d01a74d3db6761b611a9e69fe42c6d356d835877))

## [6.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v6.0.0...v6.1.0) (2026-11-13)


### Features

* **code-nav:** measure the question set per layer and record the no-vector-index ADR ([#703](https://github.com/JorisJonkers-dev/agent-kit/issues/703)) ([e657e36](https://github.com/JorisJonkers-dev/agent-kit/commit/e657e363cf069e7509374020b6b622fa7e86fdd7)), closes [#526](https://github.com/JorisJonkers-dev/agent-kit/issues/526)
* **critique:** adjudication, report and ledger lines for the Rust runner ([#706](https://github.com/JorisJonkers-dev/agent-kit/issues/706)) ([05c14b8](https://github.com/JorisJonkers-dev/agent-kit/commit/05c14b8231489cddfe2beddc6eaa886b92c01ce0)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** calibration in critique stats, and the live checks for the review stages ([#680](https://github.com/JorisJonkers-dev/agent-kit/issues/680)) ([6894016](https://github.com/JorisJonkers-dev/agent-kit/commit/689401647bf6e1fdf7836d9d08b953ae59445a2e))
* **critique:** critic prompt and the read-only tripwire for the Rust runner ([#716](https://github.com/JorisJonkers-dev/agent-kit/issues/716)) ([e3b52b2](https://github.com/JorisJonkers-dev/agent-kit/commit/e3b52b288b84a6edebd03f7dd30f28d385ebeedc))
* **critique:** evidence check, declared checks and the packet index for the Rust runner ([#713](https://github.com/JorisJonkers-dev/agent-kit/issues/713)) ([002b596](https://github.com/JorisJonkers-dev/agent-kit/commit/002b596f15baef56f97dbcf58f0532e4c1fc8549))
* **critique:** fan-out command for the Rust runner ([#724](https://github.com/JorisJonkers-dev/agent-kit/issues/724)) ([588f1f0](https://github.com/JorisJonkers-dev/agent-kit/commit/588f1f0c453d15c950ea6bb15d7d470da897463a))
* **critique:** findings and verdicts for the Rust runner ([#701](https://github.com/JorisJonkers-dev/agent-kit/issues/701)) ([710a0a6](https://github.com/JorisJonkers-dev/agent-kit/commit/710a0a651d8c756af97027af0c9475eeaf4da4cd)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port check-adrs to the Rust runner ([#689](https://github.com/JorisJonkers-dev/agent-kit/issues/689)) ([33d5029](https://github.com/JorisJonkers-dev/agent-kit/commit/33d50294f4f23de6bc2f166858bfb1a0eabb51cd)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port check-plan to the Rust runner ([#690](https://github.com/JorisJonkers-dev/agent-kit/issues/690)) ([cc71965](https://github.com/JorisJonkers-dev/agent-kit/commit/cc719658e91109ce35cfeb0907e51280ce93b359)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port check-rewrite to the Rust runner ([#691](https://github.com/JorisJonkers-dev/agent-kit/issues/691)) ([88697da](https://github.com/JorisJonkers-dev/agent-kit/commit/88697dae5fcefe3adacdd81b11c54f82e0efbdf0)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port check-tickets to the Rust runner ([#704](https://github.com/JorisJonkers-dev/agent-kit/issues/704)) ([4cab54d](https://github.com/JorisJonkers-dev/agent-kit/commit/4cab54d892636e6d1e0f0d06f981a5cca3762119)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port cold-read to the Rust runner ([#693](https://github.com/JorisJonkers-dev/agent-kit/issues/693)) ([2aa28bd](https://github.com/JorisJonkers-dev/agent-kit/commit/2aa28bdbbf57913f6fe06014bc50ed6e095f797d))
* **critique:** port facts to the Rust runner ([#700](https://github.com/JorisJonkers-dev/agent-kit/issues/700)) ([e1147eb](https://github.com/JorisJonkers-dev/agent-kit/commit/e1147eb1c3051f90ec37d55d68fb94b792fd6764)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port plan-file and the small packet readers to the Rust runner ([#709](https://github.com/JorisJonkers-dev/agent-kit/issues/709)) ([d7aa960](https://github.com/JorisJonkers-dev/agent-kit/commit/d7aa960243e0b6e49802218764501c84d58ce96a)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port reviews and the en-locale string compare to the Rust runner ([#696](https://github.com/JorisJonkers-dev/agent-kit/issues/696)) ([77f3281](https://github.com/JorisJonkers-dev/agent-kit/commit/77f328123c27c05f3282c9c9007f92860515b352))
* **critique:** port search-server to the Rust runner ([#705](https://github.com/JorisJonkers-dev/agent-kit/issues/705)) ([d571734](https://github.com/JorisJonkers-dev/agent-kit/commit/d571734b2e72d78a23b8c6dbd136b4dbdb07ddce)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port session-start and its hook to the Rust runner ([#707](https://github.com/JorisJonkers-dev/agent-kit/issues/707)) ([6265f2c](https://github.com/JorisJonkers-dev/agent-kit/commit/6265f2c58d4ae573c371a3097cb10e0bd07bcf60)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port state and budget to the Rust runner ([#715](https://github.com/JorisJonkers-dev/agent-kit/issues/715)) ([13a6899](https://github.com/JorisJonkers-dev/agent-kit/commit/13a6899993d074c0a8213c2b5f27ddeceb09bd0e)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port stats to the Rust runner ([#698](https://github.com/JorisJonkers-dev/agent-kit/issues/698)) ([49393f6](https://github.com/JorisJonkers-dev/agent-kit/commit/49393f61f32b8c9b0f34766dc91c9790d3126e65)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the ADR, decisions, recovery and architecture packet readers ([#714](https://github.com/JorisJonkers-dev/agent-kit/issues/714)) ([20c9944](https://github.com/JorisJonkers-dev/agent-kit/commit/20c9944b5253373eb24dd66b293bcc2ac45e272e)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the commit and PR gates and their hooks to the Rust runner ([#711](https://github.com/JorisJonkers-dev/agent-kit/issues/711)) ([2a79283](https://github.com/JorisJonkers-dev/agent-kit/commit/2a79283e78d58aa1a80e4a6dfe10f1f6c146c4a8)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the depth choice and roster resolution to the Rust runner ([#722](https://github.com/JorisJonkers-dev/agent-kit/issues/722)) ([9233e75](https://github.com/JorisJonkers-dev/agent-kit/commit/9233e75db857c27caf0fbc68615daea78315c629)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the prepare command to the Rust runner ([#723](https://github.com/JorisJonkers-dev/agent-kit/issues/723)) ([00be1a3](https://github.com/JorisJonkers-dev/agent-kit/commit/00be1a3f9b1d8062f826a4c902476493a9160942)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the repository tree sampler to the Rust runner ([#710](https://github.com/JorisJonkers-dev/agent-kit/issues/710)) ([663f8f9](https://github.com/JorisJonkers-dev/agent-kit/commit/663f8f909928e39abe8a18ddbf73bfe168d527d3)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the setup command to the Rust runner ([#729](https://github.com/JorisJonkers-dev/agent-kit/issues/729)) ([df95931](https://github.com/JorisJonkers-dev/agent-kit/commit/df95931ef6e6e556d0eb41c4e9cff67ca8faf6d1)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port the source graph, repo map and code sampler to the Rust runner ([#719](https://github.com/JorisJonkers-dev/agent-kit/issues/719)) ([06d312b](https://github.com/JorisJonkers-dev/agent-kit/commit/06d312b28c1bde02affe342f432e5cb5aa4880b3)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** port triage to the Rust runner, with a gh port and the tracker ([#702](https://github.com/JorisJonkers-dev/agent-kit/issues/702)) ([77959f5](https://github.com/JorisJonkers-dev/agent-kit/commit/77959f5a0978cc09c9d6bea7cda5ad14341092cd)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** read half of the flow budget for the Rust runner ([#694](https://github.com/JorisJonkers-dev/agent-kit/issues/694)) ([95a24c2](https://github.com/JorisJonkers-dev/agent-kit/commit/95a24c2723d272b2cc839bf1b20472a16a833c76)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** record command for the Rust runner ([#708](https://github.com/JorisJonkers-dev/agent-kit/issues/708)) ([83ecdff](https://github.com/JorisJonkers-dev/agent-kit/commit/83ecdffcb34c552728128d6336400e7901f48bed)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** refute command for the Rust runner ([#720](https://github.com/JorisJonkers-dev/agent-kit/issues/720)) ([2f311b6](https://github.com/JorisJonkers-dev/agent-kit/commit/2f311b6ab7df2c5f7e3b0f1d78276c50d5e57a5c))
* **critique:** run manifest, flow-budget ledger line, Vibe ask port and shared plumbing for the Rust runner ([#692](https://github.com/JorisJonkers-dev/agent-kit/issues/692)) ([a9351c3](https://github.com/JorisJonkers-dev/agent-kit/commit/a9351c3afc84431913b1677b50c6a954d11107ab))
* **critique:** run the Rust critique runner through a launcher once setup installs it ([#712](https://github.com/JorisJonkers-dev/agent-kit/issues/712)) ([3f4bbe1](https://github.com/JorisJonkers-dev/agent-kit/commit/3f4bbe16f2c3602451c539d26ef63acc618f3c15)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** Rust crate with the command shell, golden harness and allow ([#688](https://github.com/JorisJonkers-dev/agent-kit/issues/688)) ([a60a55e](https://github.com/JorisJonkers-dev/agent-kit/commit/a60a55e22800cdb264abca13a1821e6e00225166)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **critique:** the critic clients and one critic's launch for the Rust runner ([#718](https://github.com/JorisJonkers-dev/agent-kit/issues/718)) ([afb54ee](https://github.com/JorisJonkers-dev/agent-kit/commit/afb54eeead51948380ba514d4abd4da304d817a9))
* **critique:** wire the runner's search server into the critics a run launches ([#726](https://github.com/JorisJonkers-dev/agent-kit/issues/726)) ([f75340e](https://github.com/JorisJonkers-dev/agent-kit/commit/f75340eea9e5a2a87dec822e9e0f0f5f390c8da0)), closes [#498](https://github.com/JorisJonkers-dev/agent-kit/issues/498)
* **launcher:** read limit for the personal instance on the work Mac ([#667](https://github.com/JorisJonkers-dev/agent-kit/issues/667)) ([7ee9dbd](https://github.com/JorisJonkers-dev/agent-kit/commit/7ee9dbd20e85cd5a61ac3c8a26124098b6e385a3))
* **litellm:** a script that reads a key's budget window and reset from /key/info ([#672](https://github.com/JorisJonkers-dev/agent-kit/issues/672)) ([c948694](https://github.com/JorisJonkers-dev/agent-kit/commit/c948694c044da1754135ec09479ea490e00bab8a))
* **litellm:** access-group keys, a public key and resolvable gateway models ([#685](https://github.com/JorisJonkers-dev/agent-kit/issues/685)) ([5e883cb](https://github.com/JorisJonkers-dev/agent-kit/commit/5e883cbad39d0ccd43535a5fc783fed8712bac7c))
* **litellm:** send the virtual key as a standard bearer ([#683](https://github.com/JorisJonkers-dev/agent-kit/issues/683)) ([032ef7d](https://github.com/JorisJonkers-dev/agent-kit/commit/032ef7d347d0f5228f8d6650ccf61310c33228a3))
* **memory:** a bank resolver that chooses the bank by the repository's remote owner ([#728](https://github.com/JorisJonkers-dev/agent-kit/issues/728)) ([c25c0d2](https://github.com/JorisJonkers-dev/agent-kit/commit/c25c0d2acc5edfd97e2f7c59484795802918365c))
* **model-gateway:** port batch to Rust ([#681](https://github.com/JorisJonkers-dev/agent-kit/issues/681)) ([afa8f1c](https://github.com/JorisJonkers-dev/agent-kit/commit/afa8f1c7a952e7956703583ae53b9bde11595d2e)), closes [#500](https://github.com/JorisJonkers-dev/agent-kit/issues/500)
* **model-gateway:** port write-mode run to Rust ([#671](https://github.com/JorisJonkers-dev/agent-kit/issues/671)) ([f711e28](https://github.com/JorisJonkers-dev/agent-kit/commit/f711e2811d6a4306ecadb8c9dc1a62781bf0fcdc)), closes [#500](https://github.com/JorisJonkers-dev/agent-kit/issues/500)
* **model-gateway:** run the Rust gateway behind the model-gateway command ([#682](https://github.com/JorisJonkers-dev/agent-kit/issues/682)) ([d80a083](https://github.com/JorisJonkers-dev/agent-kit/commit/d80a08342042051f7e07ac9ce2e6d3752834a497)), closes [#500](https://github.com/JorisJonkers-dev/agent-kit/issues/500)
* **setup:** install the general skill sets on the agent ids that bind them ([#670](https://github.com/JorisJonkers-dev/agent-kit/issues/670)) ([fd5c086](https://github.com/JorisJonkers-dev/agent-kit/commit/fd5c08641795772f0b60345136abaf7929f7b66d))
* **setup:** revoke a host token, a per-host OpenRouter key, the Overleaf login decision ([#676](https://github.com/JorisJonkers-dev/agent-kit/issues/676)) ([f61e1d6](https://github.com/JorisJonkers-dev/agent-kit/commit/f61e1d6132bf05ba27729d8bd185cc4ae5576a8c))


### Bug Fixes

* **ci:** cancel superseded runs and skip label events ([#697](https://github.com/JorisJonkers-dev/agent-kit/issues/697)) ([491521e](https://github.com/JorisJonkers-dev/agent-kit/commit/491521ed651ead403f0337b140581df59525a4ab))
* **critique:** add the git methods two test fakes were missing ([#717](https://github.com/JorisJonkers-dev/agent-kit/issues/717)) ([f557125](https://github.com/JorisJonkers-dev/agent-kit/commit/f557125d4853bf6608b77b488137a7706838f716))
* **critique:** drop the git methods two test fakes now define twice ([#721](https://github.com/JorisJonkers-dev/agent-kit/issues/721)) ([84e4823](https://github.com/JorisJonkers-dev/agent-kit/commit/84e482395ed3ed3b2d21701b3a6e6420e02b4539))
* **hygiene:** never cancel a run of the required hygiene checks ([#677](https://github.com/JorisJonkers-dev/agent-kit/issues/677)) ([867b3fe](https://github.com/JorisJonkers-dev/agent-kit/commit/867b3fe584e2c4895b5d5cebff9c4549e8ea1f44))

## [6.0.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v5.2.0...v6.0.0) (2026-11-12)


### ⚠ BREAKING CHANGES

* the renderer is now scripts/render_agent_kit.py, and the runtime package's build paths moved from runner-manifests/runtime/ to runner/manifests/runtime/.
* the tracked .claude/, .agents/ and .council/context/ copies are gone; run `uv run python render-agent-kit.py --write` after pulling. Source paths under skills/ moved to skills/workstation/ and templates/repo/ to skills/repo/ and agents/.
* the runtime-home package drops sdd/, the speckit-seed self-test and the sddSeed contract field.

### Features

* **agent-launcher:** Landlock write limit with ADR-0013's table of writable paths ([#609](https://github.com/JorisJonkers-dev/agent-kit/issues/609)) ([96e5795](https://github.com/JorisJonkers-dev/agent-kit/commit/96e5795b7b5254a43112af44478a35a77859104c))
* **agents:** AGENTS.md and a lint for dead paths and symbols ([#595](https://github.com/JorisJonkers-dev/agent-kit/issues/595)) ([4cf6b41](https://github.com/JorisJonkers-dev/agent-kit/commit/4cf6b410d34d2841e3da3a5771182a134af7bf5a))
* **agents:** check estate skill sources for drift weekly ([#532](https://github.com/JorisJonkers-dev/agent-kit/issues/532)) ([3da4964](https://github.com/JorisJonkers-dev/agent-kit/commit/3da4964e478b8acff8acf8c21785f3a806e7e5e2))
* **build-tracker:** live ticket graph for every multi-ticket build ([#275](https://github.com/JorisJonkers-dev/agent-kit/issues/275)) ([0993045](https://github.com/JorisJonkers-dev/agent-kit/commit/0993045bc5ed0f817835c0fd91908dcf3ad104f6)), closes [#269](https://github.com/JorisJonkers-dev/agent-kit/issues/269)
* **build-tracker:** one page for every ticket of a spec, grouped by branch ([#607](https://github.com/JorisJonkers-dev/agent-kit/issues/607)) ([40b0d25](https://github.com/JorisJonkers-dev/agent-kit/commit/40b0d2552667c64ccba5b38af38aef452e00c215))
* **build-tracker:** port the build tracker to Rust ([#592](https://github.com/JorisJonkers-dev/agent-kit/issues/592)) ([9bbf3dd](https://github.com/JorisJonkers-dev/agent-kit/commit/9bbf3ddff3a3abe6b8c0c8fb405cf14887cc9253))
* **code-nav:** declare the server in the registry with personal roots ([#617](https://github.com/JorisJonkers-dev/agent-kit/issues/617)) ([ad5c233](https://github.com/JorisJonkers-dev/agent-kit/commit/ad5c233b65ae2d06d0c99b0688c2b5619c44a6a7))
* **code-nav:** frozen question set and runner ([#517](https://github.com/JorisJonkers-dev/agent-kit/issues/517)) ([#545](https://github.com/JorisJonkers-dev/agent-kit/issues/545)) ([64954f2](https://github.com/JorisJonkers-dev/agent-kit/commit/64954f2c4509dd2e6df0e4257c53401f284895d1))
* **code-nav:** outline a file and expand one symbol ([#612](https://github.com/JorisJonkers-dev/agent-kit/issues/612)) ([9333369](https://github.com/JorisJonkers-dev/agent-kit/commit/93333696a207cd7835538e58a8308724a290bc5e))
* **code-nav:** read-only search server with a root allowlist ([#589](https://github.com/JorisJonkers-dev/agent-kit/issues/589)) ([d514992](https://github.com/JorisJonkers-dev/agent-kit/commit/d514992948efcf92683138917d388b26038bd3bd))
* **code-nav:** repository map of files and symbols ranked by references ([#625](https://github.com/JorisJonkers-dev/agent-kit/issues/625)) ([68164fd](https://github.com/JorisJonkers-dev/agent-kit/commit/68164fd766e0257d4e1c01cae34afa1244e250f2))
* **code-nav:** symbols across repositories and related repositories ([#639](https://github.com/JorisJonkers-dev/agent-kit/issues/639)) ([1768348](https://github.com/JorisJonkers-dev/agent-kit/commit/17683489bb28b70aaba759302e11ec0eca68e7bf))
* **container:** kotlin-lsp, jdtls and oras in the agents image ([#638](https://github.com/JorisJonkers-dev/agent-kit/issues/638)) ([2b4f8c9](https://github.com/JorisJonkers-dev/agent-kit/commit/2b4f8c9a4f15be7ba52ffc807d96f1bd8ba08928))
* **council:** install council/ts dependencies with pnpm under a 7-day cooldown ([#586](https://github.com/JorisJonkers-dev/agent-kit/issues/586)) ([bdb3006](https://github.com/JorisJonkers-dev/agent-kit/commit/bdb30060fba216669448c9fd5878dbd36188b80a))
* **critique:** /critique-adrs core and check-adrs pre-check ([#538](https://github.com/JorisJonkers-dev/agent-kit/issues/538)) ([f311c84](https://github.com/JorisJonkers-dev/agent-kit/commit/f311c847ab8090846561ed52b5b69e4f538b1be6)), closes [#474](https://github.com/JorisJonkers-dev/agent-kit/issues/474)
* **critique:** adr-reviewer role and the ADR-record critic ([#553](https://github.com/JorisJonkers-dev/agent-kit/issues/553)) ([d996d83](https://github.com/JorisJonkers-dev/agent-kit/commit/d996d83ca57d3292753d650c90d7a49196cf7aa0))
* **critique:** architecture lenses and the design-evidence fragment ([#543](https://github.com/JorisJonkers-dev/agent-kit/issues/543)) ([dd3ea96](https://github.com/JorisJonkers-dev/agent-kit/commit/dd3ea96c5605322050927eea5744cfe7fa33bf82))
* **critique:** architecture-review stage and /review-architecture ([#613](https://github.com/JorisJonkers-dev/agent-kit/issues/613)) ([9ee976c](https://github.com/JorisJonkers-dev/agent-kit/commit/9ee976cc79b4ce60db4df57d157be6b67918de93))
* **critique:** check-rewrite and cold-read for editorial ADR rewrites ([#604](https://github.com/JorisJonkers-dev/agent-kit/issues/604)) ([4b99b16](https://github.com/JorisJonkers-dev/agent-kit/commit/4b99b16489f049688cf6f49c01aee208277b8542)), closes [#476](https://github.com/JorisJonkers-dev/agent-kit/issues/476) [#392](https://github.com/JorisJonkers-dev/agent-kit/issues/392)
* **critique:** decision-recovery stage and /recover-decisions ([#594](https://github.com/JorisJonkers-dev/agent-kit/issues/594)) ([71f4a68](https://github.com/JorisJonkers-dev/agent-kit/commit/71f4a68165c30f91bc610c9d8de3e989a90e4c52))
* **critique:** flag drifted and stale ADRs in check-adrs ([#557](https://github.com/JorisJonkers-dev/agent-kit/issues/557)) ([8896bb9](https://github.com/JorisJonkers-dev/agent-kit/commit/8896bb953aac69c671232d8be9a74cd32cfb1328))
* **critique:** history and code facts from the runner ([#562](https://github.com/JorisJonkers-dev/agent-kit/issues/562)) ([9b9b5bf](https://github.com/JorisJonkers-dev/agent-kit/commit/9b9b5bfacd2f05390af7c478721332e828443bd1))
* **critique:** order the repository stages' tree sample by code-nav's repository map ([#636](https://github.com/JorisJonkers-dev/agent-kit/issues/636)) ([7d23818](https://github.com/JorisJonkers-dev/agent-kit/commit/7d23818a620b4752666af183fe426dd11e7ae1a2))
* **critique:** plan stage, plan-critic role and plan lenses ([#619](https://github.com/JorisJonkers-dev/agent-kit/issues/619)) ([cdcb1b1](https://github.com/JorisJonkers-dev/agent-kit/commit/cdcb1b1afc82205c31bb6b1d694b3ea89a4e31d2))
* **critique:** refactoring plan format and check-plan ([#563](https://github.com/JorisJonkers-dev/agent-kit/issues/563)) ([19c32c3](https://github.com/JorisJonkers-dev/agent-kit/commit/19c32c38aff796b220057bbb567347ee9c2ae7fa))
* **critique:** wire the work-only critic launchers into the critique runner ([#646](https://github.com/JorisJonkers-dev/agent-kit/issues/646)) ([91796b4](https://github.com/JorisJonkers-dev/agent-kit/commit/91796b42512b9622a84763377ad281a0e47550cf))
* **critique:** work-only critic roster for nedap repositories ([#549](https://github.com/JorisJonkers-dev/agent-kit/issues/549)) ([da035f9](https://github.com/JorisJonkers-dev/agent-kit/commit/da035f9bac70e26f314f8fa9f4e4aa069d18e4a8))
* **gateway:** Codex worktree workers can commit ([#601](https://github.com/JorisJonkers-dev/agent-kit/issues/601)) ([4a79327](https://github.com/JorisJonkers-dev/agent-kit/commit/4a793279e39e02196f6dbab02121c0be2ae2f88a))
* **gold-band:** critique stage templates over the runner ([#620](https://github.com/JorisJonkers-dev/agent-kit/issues/620)) ([e0ba0fa](https://github.com/JorisJonkers-dev/agent-kit/commit/e0ba0fa107aeddbbaecaef9e1e7e42056832c979))
* **gold-band:** finding check node keeps only findings with checkable evidence ([#626](https://github.com/JorisJonkers-dev/agent-kit/issues/626)) ([ece7302](https://github.com/JorisJonkers-dev/agent-kit/commit/ece7302b4d7e796519b57deb703473ee83320e86))
* **gold-band:** gate shim ([#599](https://github.com/JorisJonkers-dev/agent-kit/issues/599)) ([ecc4ae4](https://github.com/JorisJonkers-dev/agent-kit/commit/ecc4ae4ef65fac5c025d554bd9b02a5ca9cf3632)), closes [#445](https://github.com/JorisJonkers-dev/agent-kit/issues/445)
* **gold-band:** grant-check node reserves one Codex node start ([#659](https://github.com/JorisJonkers-dev/agent-kit/issues/659)) ([83f1016](https://github.com/JorisJonkers-dev/agent-kit/commit/83f1016d9f2bd4a0047d39b986f2af5257b4708d))
* **gold-band:** headless proof harness and refusal of unproven runs ([#603](https://github.com/JorisJonkers-dev/agent-kit/issues/603)) ([4c04a10](https://github.com/JorisJonkers-dev/agent-kit/commit/4c04a108892e23cfaa39b0bdfe73e10c00290f0e))
* **gold-band:** run-start shim ([#593](https://github.com/JorisJonkers-dev/agent-kit/issues/593)) ([32f84f5](https://github.com/JorisJonkers-dev/agent-kit/commit/32f84f5d1aa44d537c64579d7a96e91732fad1db)), closes [#444](https://github.com/JorisJonkers-dev/agent-kit/issues/444)
* **gold-band:** Wave 4 human-gate workflows ([#647](https://github.com/JorisJonkers-dev/agent-kit/issues/647)) ([ef68155](https://github.com/JorisJonkers-dev/agent-kit/commit/ef68155f0464ebb2f209a4a92212f4425f3d392b))
* **gold-band:** Workflow B critics, unrolled repairs and escalation ([#640](https://github.com/JorisJonkers-dev/agent-kit/issues/640)) ([f72ba07](https://github.com/JorisJonkers-dev/agent-kit/commit/f72ba07e8713884399ed2c541ea797e5d269aa4b))
* **gold-band:** Workflow B in the work instance with a grant-check node before every Codex node ([#662](https://github.com/JorisJonkers-dev/agent-kit/issues/662)) ([938f50b](https://github.com/JorisJonkers-dev/agent-kit/commit/938f50be808bce4fff26cdf9e7ca1a269320b005)), closes [#392](https://github.com/JorisJonkers-dev/agent-kit/issues/392)
* **gold-band:** Workflow B main path with stubs, proven headless ([#611](https://github.com/JorisJonkers-dev/agent-kit/issues/611)) ([b8921cd](https://github.com/JorisJonkers-dev/agent-kit/commit/b8921cd06489194c6f430730b72affed3b91a4ed))
* **gold-band:** Workflow C in the personal instance ([#634](https://github.com/JorisJonkers-dev/agent-kit/issues/634)) ([5945848](https://github.com/JorisJonkers-dev/agent-kit/commit/59458480cfbfbe11c003026c142b8dc7d009912d))
* **gold-band:** Workflow C in the work instance ([#663](https://github.com/JorisJonkers-dev/agent-kit/issues/663)) ([8a7b35d](https://github.com/JorisJonkers-dev/agent-kit/commit/8a7b35de0f97101d95163cfdf1d930e15e2955c0))
* **gold-band:** workflows over the runners for critique, stats, flow and build tracker ([#645](https://github.com/JorisJonkers-dev/agent-kit/issues/645)) ([24de852](https://github.com/JorisJonkers-dev/agent-kit/commit/24de8528c8c48bac1bac6523b5f4fe2ee3e73ed8))
* **inventory:** cut the waves after Wave 2 and check them ([#632](https://github.com/JorisJonkers-dev/agent-kit/issues/632)) ([2fff334](https://github.com/JorisJonkers-dev/agent-kit/commit/2fff334278376889968ff72f0dadcc009bafcf84))
* **inventory:** port inventory with lanes and its check ([#542](https://github.com/JorisJonkers-dev/agent-kit/issues/542)) ([0d1bdc8](https://github.com/JorisJonkers-dev/agent-kit/commit/0d1bdc8bcb8c1520e8d34ea5d5341ff1bb4710d8))
* **kube:** a port-forward-only context for the scheduled port-forward agent ([#577](https://github.com/JorisJonkers-dev/agent-kit/issues/577)) ([48a7901](https://github.com/JorisJonkers-dev/agent-kit/commit/48a7901c45071beddec0483310d2741aeae56618))
* **kubernetes:** run kubernetes-mcp-server locally over stdio; retire the port-forward and in-cluster Hermes ([#578](https://github.com/JorisJonkers-dev/agent-kit/issues/578)) ([d8ce8f6](https://github.com/JorisJonkers-dev/agent-kit/commit/d8ce8f68384c58af60fc4e7105adfa41bc7904f2))
* **launcher:** reach-level launcher with a per-OS support record ([#588](https://github.com/JorisJonkers-dev/agent-kit/issues/588)) ([95c4b9a](https://github.com/JorisJonkers-dev/agent-kit/commit/95c4b9aa32f748e3a46079c9e3b4b8d369085eeb))
* **launcher:** sandbox-exec write limit on macOS ([#637](https://github.com/JorisJonkers-dev/agent-kit/issues/637)) ([f5e2f0e](https://github.com/JorisJonkers-dev/agent-kit/commit/f5e2f0ea41f7b4119c0d481957c65db1f2e03dd3))
* **launcher:** verify Pi, OpenCode, Hermes and Antigravity reach-level cells per OS ([#656](https://github.com/JorisJonkers-dev/agent-kit/issues/656)) ([912c42a](https://github.com/JorisJonkers-dev/agent-kit/commit/912c42ab964026d46147d2738bfa464d4c0a4c1b))
* **launcher:** verify the Claude reach-level cells on Linux and macOS ([#654](https://github.com/JorisJonkers-dev/agent-kit/issues/654)) ([94b4d0f](https://github.com/JorisJonkers-dev/agent-kit/commit/94b4d0fcda1754f5395a6889291d35c14a4c275f))
* **launcher:** verify the Vibe reach-level cells on Linux and macOS ([#657](https://github.com/JorisJonkers-dev/agent-kit/issues/657)) ([dab5209](https://github.com/JorisJonkers-dev/agent-kit/commit/dab5209fac35f2e415a6a2eee3b6c787cbfdc3f4))
* **litellm:** per-key budgets, wildcard routes and the litellm-gate check ([#565](https://github.com/JorisJonkers-dev/agent-kit/issues/565)) ([28545d4](https://github.com/JorisJonkers-dev/agent-kit/commit/28545d47109b5ef130e5af6d074b09b066575331))
* model gateway for every delegation ([#325](https://github.com/JorisJonkers-dev/agent-kit/issues/325)) ([e8cad85](https://github.com/JorisJonkers-dev/agent-kit/commit/e8cad85c613a362d2087c644a711663d77b68892))
* **model-gateway:** binding family and stated escalation ([#535](https://github.com/JorisJonkers-dev/agent-kit/issues/535)) ([8e6eeb8](https://github.com/JorisJonkers-dev/agent-kit/commit/8e6eeb8ef5bfb4cca46a32608a9d78bde20bc432))
* **model-gateway:** port grant reserve and worktrees clean to Rust ([#661](https://github.com/JorisJonkers-dev/agent-kit/issues/661)) ([78e34cd](https://github.com/JorisJonkers-dev/agent-kit/commit/78e34cd3e8129f9ff39b49f82a9428f949d4bdfb))
* **model-gateway:** port models, usage, grant write and grant list to Rust ([#660](https://github.com/JorisJonkers-dev/agent-kit/issues/660)) ([4e6fa45](https://github.com/JorisJonkers-dev/agent-kit/commit/4e6fa4504d9324fbd244385ecd85ec5d8cc90231)), closes [#500](https://github.com/JorisJonkers-dev/agent-kit/issues/500)
* **model-gateway:** port read-only run to Rust ([#665](https://github.com/JorisJonkers-dev/agent-kit/issues/665)) ([9caeb00](https://github.com/JorisJonkers-dev/agent-kit/commit/9caeb0025de6c79bca3359f1f03764635721ecf3))
* **opencode:** install and configure OpenCode on the workstation ([#313](https://github.com/JorisJonkers-dev/agent-kit/issues/313)) ([4421dd7](https://github.com/JorisJonkers-dev/agent-kit/commit/4421dd7368590ef465f1aaf5e27f34e8bddae1a0))
* **projects-layout:** one place for clones and worktrees, with a guard hook ([#376](https://github.com/JorisJonkers-dev/agent-kit/issues/376)) ([d154633](https://github.com/JorisJonkers-dev/agent-kit/commit/d154633cec5b3f1b0cb563f64a289fa9a8db356c))
* **registry:** add the Grimoire MCP server for the workstation ([#267](https://github.com/JorisJonkers-dev/agent-kit/issues/267)) ([a4ceb65](https://github.com/JorisJonkers-dev/agent-kit/commit/a4ceb65955cc2366664fedb140597306ba942fd5))
* **registry:** list dig, file and binutils for NixOS workstations ([#582](https://github.com/JorisJonkers-dev/agent-kit/issues/582)) ([a9a2af8](https://github.com/JorisJonkers-dev/agent-kit/commit/a9a2af85c60bcecdfd7e2a66415c8d2e9ed79475))
* **registry:** model both Gold Band instances and the overlay ([#544](https://github.com/JorisJonkers-dev/agent-kit/issues/544)) ([92ff941](https://github.com/JorisJonkers-dev/agent-kit/commit/92ff94183109a337180335844559931294e1abc1))
* **registry:** render the NixOS workstation package list ([#576](https://github.com/JorisJonkers-dev/agent-kit/issues/576)) ([d019e35](https://github.com/JorisJonkers-dev/agent-kit/commit/d019e351e8214eb67e70ad83d34000ad91e3374f))
* **runner:** overlay the newest published kit at session start ([#658](https://github.com/JorisJonkers-dev/agent-kit/issues/658)) ([57f2420](https://github.com/JorisJonkers-dev/agent-kit/commit/57f2420184aa8b43246572aa817ee2ae60399f8f))
* **rust:** cargo workspace and the 7-day crate cooldown on Linux ([#536](https://github.com/JorisJonkers-dev/agent-kit/issues/536)) ([d3b5944](https://github.com/JorisJonkers-dev/agent-kit/commit/d3b5944ab3f1de8ae2da04b9f95d0361c7825cbe))
* **rust:** CI-wait gate with a structured verdict and a bounded flaky rerun ([#610](https://github.com/JorisJonkers-dev/agent-kit/issues/610)) ([b0f3978](https://github.com/JorisJonkers-dev/agent-kit/commit/b0f3978c11f4873b85ff2e1d60d6a46902f81013)), closes [#453](https://github.com/JorisJonkers-dev/agent-kit/issues/453)
* **rust:** coverage gate on changed files for Workflow B ([#633](https://github.com/JorisJonkers-dev/agent-kit/issues/633)) ([35ba4b2](https://github.com/JorisJonkers-dev/agent-kit/commit/35ba4b2d426d934f691ebd53fc6ebe603446f3e8))
* **rust:** export a published issue to a requirement file ([#598](https://github.com/JorisJonkers-dev/agent-kit/issues/598)) ([1c792d1](https://github.com/JorisJonkers-dev/agent-kit/commit/1c792d1ddb065740835a833d32b580f6d7d0efbf)), closes [#392](https://github.com/JorisJonkers-dev/agent-kit/issues/392)
* **rust:** model-gateway crate with the command shell and workflow commands ([#653](https://github.com/JorisJonkers-dev/agent-kit/issues/653)) ([16272d8](https://github.com/JorisJonkers-dev/agent-kit/commit/16272d84740cfcd01434cc11e18efe212d4bee67))
* **rust:** transcript tool-mix analyser ([#587](https://github.com/JorisJonkers-dev/agent-kit/issues/587)) ([de66dd4](https://github.com/JorisJonkers-dev/agent-kit/commit/de66dd42e1b084e4f8d428522993bb0acfcbb734))
* **setup:** apply the work overlay's code-nav roots ([#652](https://github.com/JorisJonkers-dev/agent-kit/issues/652)) ([5c9bce0](https://github.com/JorisJonkers-dev/agent-kit/commit/5c9bce098430df1844ffd538ef87c7a4efabeb45))
* **setup:** atomic Gold Band settings writes and owner-only data directory ([#635](https://github.com/JorisJonkers-dev/agent-kit/issues/635)) ([a2b4748](https://github.com/JorisJonkers-dev/agent-kit/commit/a2b47489d320b1fdadc0462f593977d3bae2e131))
* **setup:** build the pinned Gold Band on Linux ([#561](https://github.com/JorisJonkers-dev/agent-kit/issues/561)) ([5fff294](https://github.com/JorisJonkers-dev/agent-kit/commit/5fff294602e6e92fb6cab4e05e5fb43c02e15c56))
* **setup:** Claude agents in each Gold Band instance with a fixed configuration directory ([#623](https://github.com/JorisJonkers-dev/agent-kit/issues/623)) ([926e199](https://github.com/JorisJonkers-dev/agent-kit/commit/926e199c20f49cd4f03dca99ac5b2f5a34b1145f))
* **setup:** converge the personal Gold Band instance's agent map, OpenCode first ([#597](https://github.com/JorisJonkers-dev/agent-kit/issues/597)) ([fc3fbbb](https://github.com/JorisJonkers-dev/agent-kit/commit/fc3fbbbac0554ced77debe60499a308155a46f2b))
* **setup:** converge the work instance on the Mac from the overlay ([#655](https://github.com/JorisJonkers-dev/agent-kit/issues/655)) ([32156ff](https://github.com/JorisJonkers-dev/agent-kit/commit/32156ffb62ac62ce585f070f03c528f3792b7b4c))
* **setup:** declare Antigravity per OS and Hermes in Gold Band ([#650](https://github.com/JorisJonkers-dev/agent-kit/issues/650)) ([2aaeb96](https://github.com/JorisJonkers-dev/agent-kit/commit/2aaeb9638010aaf66d1384a4003a343fc3aa5494))
* **setup:** declare the estate skill set and bind it to the personal instance's agents ([#622](https://github.com/JorisJonkers-dev/agent-kit/issues/622)) ([c32f2c6](https://github.com/JorisJonkers-dev/agent-kit/commit/c32f2c6433cd0d264f65a4e4db6bcba64113823a))
* **setup:** declare Vibe and Vibe-study in the personal Gold Band instance ([#615](https://github.com/JorisJonkers-dev/agent-kit/issues/615)) ([b931c0f](https://github.com/JorisJonkers-dev/agent-kit/commit/b931c0f8629994b3f938c2e81d6d696d79705c31))
* **setup:** give OpenCode the gateway, delegation and research skills ([#608](https://github.com/JorisJonkers-dev/agent-kit/issues/608)) ([b0c7c7b](https://github.com/JorisJonkers-dev/agent-kit/commit/b0c7c7b8ec0eeef9a2ee5a268f5321c8c5fc3e46))
* **setup:** give the Gradle wrapper the package proxy's credentials ([#575](https://github.com/JorisJonkers-dev/agent-kit/issues/575)) ([61d21a0](https://github.com/JorisJonkers-dev/agent-kit/commit/61d21a05538583004acd250c058d7413c5c9d4b6))
* **setup:** install and configure Antigravity on Linux ([#651](https://github.com/JorisJonkers-dev/agent-kit/issues/651)) ([90b3291](https://github.com/JorisJonkers-dev/agent-kit/commit/90b3291290ed1babd46e3d24cbd19c1be3dc18a3))
* **setup:** install Pi from pins on both OSes and declare it in Gold Band ([#643](https://github.com/JorisJonkers-dev/agent-kit/issues/643)) ([d681e09](https://github.com/JorisJonkers-dev/agent-kit/commit/d681e096e886369dc500e83bd08d5525f65a190d))
* **setup:** Linux equivalents or reported skips for macOS-only setup paths ([#546](https://github.com/JorisJonkers-dev/agent-kit/issues/546)) ([9299bd5](https://github.com/JorisJonkers-dev/agent-kit/commit/9299bd544d0c78d73bee10b4f242f60bbec3b4a8))
* **setup:** one secret store, Keychain on macOS and libsecret on Linux ([#539](https://github.com/JorisJonkers-dev/agent-kit/issues/539)) ([510b477](https://github.com/JorisJonkers-dev/agent-kit/commit/510b477e431c81dadabe463512892c0e0c5f4418))
* **setup:** per-repository setup file and its setup command ([#534](https://github.com/JorisJonkers-dev/agent-kit/issues/534)) ([963bf4d](https://github.com/JorisJonkers-dev/agent-kit/commit/963bf4da995960f40c41dd545c1da394770f168c))
* **setup:** point every package manager at the packages proxy ([a1f8887](https://github.com/JorisJonkers-dev/agent-kit/commit/a1f88876a5fbed536d69eabe51ccc882f9527ed4))
* **setup:** point every package manager at the packages proxy ([#398](https://github.com/JorisJonkers-dev/agent-kit/issues/398)) ([a1f8887](https://github.com/JorisJonkers-dev/agent-kit/commit/a1f88876a5fbed536d69eabe51ccc882f9527ed4))
* **setup:** register the Wave 6 MCP servers in the personal Gold Band instance ([#644](https://github.com/JorisJonkers-dev/agent-kit/issues/644)) ([a965f26](https://github.com/JorisJonkers-dev/agent-kit/commit/a965f26be2958fcb0a342547b7820b2253a95085))
* **setup:** render the declared roles into each Gold Band instance's profiles ([#614](https://github.com/JorisJonkers-dev/agent-kit/issues/614)) ([25f99f1](https://github.com/JorisJonkers-dev/agent-kit/commit/25f99f109a00889cbbf2efe1e47015b60520ccc8))
* **setup:** resolve Python dependencies with a 7-day window ([#559](https://github.com/JorisJonkers-dev/agent-kit/issues/559)) ([772bc6a](https://github.com/JorisJonkers-dev/agent-kit/commit/772bc6a7a0cc177f3ce3f5f37de739aa8e7c8a94))
* **setup:** running-instance guard on Linux and macOS ([#616](https://github.com/JorisJonkers-dev/agent-kit/issues/616)) ([c3077ee](https://github.com/JorisJonkers-dev/agent-kit/commit/c3077ee7b84538c61a9d0294569d08f47ff9ca3c))
* **setup:** set a default Rust toolchain on NixOS ([#580](https://github.com/JorisJonkers-dev/agent-kit/issues/580)) ([3ff936d](https://github.com/JorisJonkers-dev/agent-kit/commit/3ff936d6f8c0a0d785a3d5c6fc79ce78d791f356))
* **setup:** skill sets per agent id ([#618](https://github.com/JorisJonkers-dev/agent-kit/issues/618)) ([ef6c6cb](https://github.com/JorisJonkers-dev/agent-kit/commit/ef6c6cb079c9cabb2695244f214f4578014df1d2))
* **setup:** skip what Nix provides on NixOS and name copies that shadow it ([#579](https://github.com/JorisJonkers-dev/agent-kit/issues/579)) ([3594a33](https://github.com/JorisJonkers-dev/agent-kit/commit/3594a33a18356f4644da6554e027a4af5418b055))
* **setup:** workstation kubeconfig helper with per-node read-only credentials ([#564](https://github.com/JorisJonkers-dev/agent-kit/issues/564)) ([ba16d3d](https://github.com/JorisJonkers-dev/agent-kit/commit/ba16d3d406540adfa05616e0d3658878b85f0068))
* **skills:** add a research front-door skill ([#596](https://github.com/JorisJonkers-dev/agent-kit/issues/596)) ([dc88f6f](https://github.com/JorisJonkers-dev/agent-kit/commit/dc88f6f05a7c5d161888323a219a61b334af8d82))
* **skills:** estate-bootstrap, start or audit a repo against the estate templates ([#552](https://github.com/JorisJonkers-dev/agent-kit/issues/552)) ([e9d43ca](https://github.com/JorisJonkers-dev/agent-kit/commit/e9d43ca8d9af8c1bd44c356578b948f85954d042))
* **skills:** estate-delivery-status, did my release land ([#550](https://github.com/JorisJonkers-dev/agent-kit/issues/550)) ([9e5480b](https://github.com/JorisJonkers-dev/agent-kit/commit/9e5480b0d2676a267a87a4d2b4cda82c3d88db1c))
* **skills:** estate-project-intent, write and validate a Project's Intent ([#548](https://github.com/JorisJonkers-dev/agent-kit/issues/548)) ([e535a89](https://github.com/JorisJonkers-dev/agent-kit/commit/e535a89cbf477d665aefa724877a6d57d3a865fe))
* **skills:** estate-rollback, plan a pause, rollback or resume ([#551](https://github.com/JorisJonkers-dev/agent-kit/issues/551)) ([24d6cfe](https://github.com/JorisJonkers-dev/agent-kit/commit/24d6cfe8b4af37b860130cdb2d16bde2ecc8b0c6))
* **skills:** retire the six old repo skills ([#621](https://github.com/JorisJonkers-dev/agent-kit/issues/621)) ([9d66646](https://github.com/JorisJonkers-dev/agent-kit/commit/9d666469982df70ddfe5fc4dc4cbc25ae3180ebf))
* **ticket-sync:** close superseded and delivered issues only under an approved plan ([#606](https://github.com/JorisJonkers-dev/agent-kit/issues/606)) ([7d6da79](https://github.com/JorisJonkers-dev/agent-kit/commit/7d6da7994d4f0541f9490eb1084aa51d0ffb5fcc))
* **tickets:** ticket sync tool with a dry run ([#554](https://github.com/JorisJonkers-dev/agent-kit/issues/554)) ([452aada](https://github.com/JorisJonkers-dev/agent-kit/commit/452aadacf8dea3a096b5e3cf62f45dd07c2d5c30))
* **workflow-lint:** refuse Opus, memory writes and loops outside the Gold Band rules ([#547](https://github.com/JorisJonkers-dev/agent-kit/issues/547)) ([725bb85](https://github.com/JorisJonkers-dev/agent-kit/commit/725bb850f653606e4ea98865259f14e36f7cd982))


### Bug Fixes

* **critique:** detect Vibe sign-in on Linux ([#394](https://github.com/JorisJonkers-dev/agent-kit/issues/394)) ([ebc6d0b](https://github.com/JorisJonkers-dev/agent-kit/commit/ebc6d0b4e26182cb194ac9023949b4a858b52e19))
* **deps:** pin dependencies ([#70](https://github.com/JorisJonkers-dev/agent-kit/issues/70)) ([d10349a](https://github.com/JorisJonkers-dev/agent-kit/commit/d10349a63931de55966969217819be4fee6751cd))
* **deps:** update minor and patch dependencies ([#649](https://github.com/JorisJonkers-dev/agent-kit/issues/649)) ([c050573](https://github.com/JorisJonkers-dev/agent-kit/commit/c05057370ed7878be97877ba574ae057652438b2))
* **hooks:** skip commits a remote branch already holds on a force-push ([#585](https://github.com/JorisJonkers-dev/agent-kit/issues/585)) ([db3deee](https://github.com/JorisJonkers-dev/agent-kit/commit/db3deee0635b888b286e68eb3f5671ffb9accf02))
* **inventory:** add the estate-project-intent row and keep personal servers out of the work instance ([#566](https://github.com/JorisJonkers-dev/agent-kit/issues/566)) ([06c6d01](https://github.com/JorisJonkers-dev/agent-kit/commit/06c6d010f3e0026c652ade5cf4f7715547f072c3))
* **lsp:** install language servers that work on NixOS, and fix kotlin-lsp and jdtls ([#375](https://github.com/JorisJonkers-dev/agent-kit/issues/375)) ([d616478](https://github.com/JorisJonkers-dev/agent-kit/commit/d61647864e769807c91ce10252b9ffb3b4a61b2b))
* **lsp:** move kotlin-lsp off the expired 263.4702.0 build ([#581](https://github.com/JorisJonkers-dev/agent-kit/issues/581)) ([a510b41](https://github.com/JorisJonkers-dev/agent-kit/commit/a510b416f2c9282275ca05b9ad89955110236fd2))
* **model-gateway:** list only the Vibe models Vibe accepts ([#391](https://github.com/JorisJonkers-dev/agent-kit/issues/391)) ([68707b8](https://github.com/JorisJonkers-dev/agent-kit/commit/68707b8e0f11fb14320f25a2484d4af9326bd3b3)), closes [#380](https://github.com/JorisJonkers-dev/agent-kit/issues/380)
* **model-gateway:** Vibe headless runs fail fast without web tools ([#540](https://github.com/JorisJonkers-dev/agent-kit/issues/540)) ([021ff67](https://github.com/JorisJonkers-dev/agent-kit/commit/021ff67d6518bfa72e6d91be971069da8847924b))
* **plugins:** update anthropics/claude-plugins-official digest to 43fd1f9 ([#583](https://github.com/JorisJonkers-dev/agent-kit/issues/583)) ([c26037b](https://github.com/JorisJonkers-dev/agent-kit/commit/c26037b8a1b9b7e105119830cec85fc36d674172))
* **plugins:** update anthropics/claude-plugins-official digest to dceb15d ([#148](https://github.com/JorisJonkers-dev/agent-kit/issues/148)) ([418c438](https://github.com/JorisJonkers-dev/agent-kit/commit/418c4387d0f9f4dc1be1cbdb403c61b251e25fc6))
* **plugins:** update anthropics/claude-plugins-official digest to e0ab6e5 ([#648](https://github.com/JorisJonkers-dev/agent-kit/issues/648)) ([5b0266f](https://github.com/JorisJonkers-dev/agent-kit/commit/5b0266fc61b522e658a97de967409d4966c8b573))
* **plugins:** update codenamev/ai-software-architect digest to 73d6f9a ([#152](https://github.com/JorisJonkers-dev/agent-kit/issues/152)) ([088ac6e](https://github.com/JorisJonkers-dev/agent-kit/commit/088ac6e22ac7d15034e420101756e447b9b38a77))
* **plugins:** update dependency juliusbrussee/caveman to v2.7.0 ([#156](https://github.com/JorisJonkers-dev/agent-kit/issues/156)) ([ac79edb](https://github.com/JorisJonkers-dev/agent-kit/commit/ac79edb9a41896a171165bf7f7d0d087bb0d1450))
* **plugins:** update jgraph/drawio-mcp digest to cfc1b91 ([#153](https://github.com/JorisJonkers-dev/agent-kit/issues/153)) ([ace2f10](https://github.com/JorisJonkers-dev/agent-kit/commit/ace2f10483a828b4557c7f490de0bae9411f037b))
* **setup:** allow only the owner's identity in commits, refuse every AI agent ([#366](https://github.com/JorisJonkers-dev/agent-kit/issues/366)) ([f08e10c](https://github.com/JorisJonkers-dev/agent-kit/commit/f08e10c9bcbed88ff2bd5cab6ca6dc30d4d80962))
* **setup:** install the work Claude profile into ~/.claude whatever CLAUDE_CONFIG_DIR says ([#537](https://github.com/JorisJonkers-dev/agent-kit/issues/537)) ([73b2cf7](https://github.com/JorisJonkers-dev/agent-kit/commit/73b2cf7f36a271780d0886317c6a8acc665ce1c4))
* **setup:** keep the init script within the line-length limit ([bd923db](https://github.com/JorisJonkers-dev/agent-kit/commit/bd923db0bb39cafd62d5f48af080a2a0407dee6d))
* **setup:** refuse a credit split across a line break in the attribution hook ([#533](https://github.com/JorisJonkers-dev/agent-kit/issues/533)) ([a059e74](https://github.com/JorisJonkers-dev/agent-kit/commit/a059e742001f27ba75f65ccfd7d372171527e0b0))
* **setup:** rewrite Gradle repository URLs to the packages proxy in p… ([bd923db](https://github.com/JorisJonkers-dev/agent-kit/commit/bd923db0bb39cafd62d5f48af080a2a0407dee6d))
* **setup:** rewrite Gradle repository URLs to the packages proxy in place ([#402](https://github.com/JorisJonkers-dev/agent-kit/issues/402)) ([bd923db](https://github.com/JorisJonkers-dev/agent-kit/commit/bd923db0bb39cafd62d5f48af080a2a0407dee6d))
* **test:** keep the test suite out of the real home ([#567](https://github.com/JorisJonkers-dev/agent-kit/issues/567)) ([7455b1f](https://github.com/JorisJonkers-dev/agent-kit/commit/7455b1f8beeb69eae448ecf34117828a192b2c90))
* **tests:** serialise fake-executable writes and spawns to end ETXTBSY flakes ([#602](https://github.com/JorisJonkers-dev/agent-kit/issues/602)) ([0e1dca3](https://github.com/JorisJonkers-dev/agent-kit/commit/0e1dca3cd5dbdfc023bbc971ebebbf7b4154066b))
* **test:** stop the grants-in-repository unhandled rejection in the adapter tests ([#641](https://github.com/JorisJonkers-dev/agent-kit/issues/641)) ([52be77b](https://github.com/JorisJonkers-dev/agent-kit/commit/52be77b420aa83cb08b0be4fe0d883e0b82b5023))
* use the .gold-band data directory under GOLD_BAND_HOME ([#624](https://github.com/JorisJonkers-dev/agent-kit/issues/624)) ([0d7a3fe](https://github.com/JorisJonkers-dev/agent-kit/commit/0d7a3fe6ecee6ba2fb362aa258b8bcc041e9a260))


### Code Refactoring

* drop Spec Kit, BMAD sync, spec-compare and claude-worker ([#274](https://github.com/JorisJonkers-dev/agent-kit/issues/274)) ([8a4fd73](https://github.com/JorisJonkers-dev/agent-kit/commit/8a4fd739e1b31d718c4288f0c93333669b9f16f5)), closes [#253](https://github.com/JorisJonkers-dev/agent-kit/issues/253)
* group the repository root into runner/, scripts/, provenance/ and docs/ ([#283](https://github.com/JorisJonkers-dev/agent-kit/issues/283)) ([f2b8870](https://github.com/JorisJonkers-dev/agent-kit/commit/f2b8870bb35a26e911eb52b66a2a99d394b7c0ad)), closes [#280](https://github.com/JorisJonkers-dev/agent-kit/issues/280) [#281](https://github.com/JorisJonkers-dev/agent-kit/issues/281) [#277](https://github.com/JorisJonkers-dev/agent-kit/issues/277)
* move skill sources under skills/ and stop tracking .claude, .agents and .council ([#282](https://github.com/JorisJonkers-dev/agent-kit/issues/282)) ([58e014a](https://github.com/JorisJonkers-dev/agent-kit/commit/58e014acba1e22278ee3aafb3bbf0840d5433862)), closes [#277](https://github.com/JorisJonkers-dev/agent-kit/issues/277)

## [5.2.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v5.1.0...v5.2.0) (2026-11-02)


### Features

* **critique:** spec [#214](https://github.com/JorisJonkers-dev/agent-kit/issues/214) phase 0, runner groundwork for review stages ([#252](https://github.com/JorisJonkers-dev/agent-kit/issues/252)) ([e8ded6e](https://github.com/JorisJonkers-dev/agent-kit/commit/e8ded6e468df848d1f560a74af1eec5f6cf81c9f))
* **registry:** add the Outline wiki MCP server for the workstation ([#264](https://github.com/JorisJonkers-dev/agent-kit/issues/264)) ([5ab891f](https://github.com/JorisJonkers-dev/agent-kit/commit/5ab891f7e87e7ea6bb20b787174ac30d4f436a32))


### Bug Fixes

* **critique:** check agy sign-in with agy models, not the Keychain ([#210](https://github.com/JorisJonkers-dev/agent-kit/issues/210)) ([137e481](https://github.com/JorisJonkers-dev/agent-kit/commit/137e4811f865dc7fb893b03ec1fbb317cb309bc0))
* **critique:** raise the ticket critics' cap to $1 and 600 s ([#248](https://github.com/JorisJonkers-dev/agent-kit/issues/248)) ([6cd0f41](https://github.com/JorisJonkers-dev/agent-kit/commit/6cd0f41bd027ee74d71167d3f98097b32432d66c))
* **runner:** keep the entrypoint's home config inside the runner container ([#262](https://github.com/JorisJonkers-dev/agent-kit/issues/262)) ([4de7b69](https://github.com/JorisJonkers-dev/agent-kit/commit/4de7b691eda352ac34a68963fa8a915686f6d4c1))

## [5.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v5.0.0...v5.1.0) (2026-11-01)


### Features

* **critique:** /critique-decisions, /check-tickets, /review-ticket and /review-feature ([#205](https://github.com/JorisJonkers-dev/agent-kit/issues/205)) ([ef2ca46](https://github.com/JorisJonkers-dev/agent-kit/commit/ef2ca46b7663f1556aac9aeb6c86d608b5620f3e))
* **critique:** /flow, hooks and /critique-stats ([#202](https://github.com/JorisJonkers-dev/agent-kit/issues/202)) ([0e13e79](https://github.com/JorisJonkers-dev/agent-kit/commit/0e13e790707f1d0b4b6041403c34c79cec6208c0))
* **critique:** critic families, tripwire, budget, depth and /critique-spec ([#203](https://github.com/JorisJonkers-dev/agent-kit/issues/203)) ([914b80f](https://github.com/JorisJonkers-dev/agent-kit/commit/914b80f03e68d6777010cb1a8bfac62d4e9d3726))


### Bug Fixes

* **installer:** serve the installer and its kit bundle from GitHub releases ([#206](https://github.com/JorisJonkers-dev/agent-kit/issues/206)) ([7c5fd29](https://github.com/JorisJonkers-dev/agent-kit/commit/7c5fd2970a1da891c70c8fd265c439b7ef92379c))

## [5.0.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.16.0...v5.0.0) (2026-11-01)


### ⚠ BREAKING CHANGES

* **vibe:** `memory-private claude|codex` is removed. Claude Code and Codex can no longer reach Private Memory; setup deletes the Claude --mcp-config file and Codex's switched-off entry. Plain `vibe` no longer asks for the passphrase and has no Private Memory: use `vibe-private`.

### Features

* **antigravity:** install and configure Google Antigravity ([#166](https://github.com/JorisJonkers-dev/agent-kit/issues/166)) ([3f402e8](https://github.com/JorisJonkers-dev/agent-kit/commit/3f402e82147ff5abeba0e68df95e7fa49e6adacf))
* **critique:** runner tracer, one Vibe critic reviews a local-markdown spec ([#180](https://github.com/JorisJonkers-dev/agent-kit/issues/180)) ([7b3641a](https://github.com/JorisJonkers-dev/agent-kit/commit/7b3641a9152b26fcda67f5d81f9a2b6861c8aa89)), closes [#169](https://github.com/JorisJonkers-dev/agent-kit/issues/169)
* **vibe:** sign vibe-study in with Vibe's own browser sign-in ([#160](https://github.com/JorisJonkers-dev/agent-kit/issues/160)) ([78bdf95](https://github.com/JorisJonkers-dev/agent-kit/commit/78bdf95b601f928c045b2e4ce5e4eeafd3132b29)), closes [#157](https://github.com/JorisJonkers-dev/agent-kit/issues/157)
* **vibe:** split vibe into vibe, vibe-study and vibe-private ([#159](https://github.com/JorisJonkers-dev/agent-kit/issues/159)) ([95ae50e](https://github.com/JorisJonkers-dev/agent-kit/commit/95ae50e7e52c012f4910ee875b02e83bde6b1cfb)), closes [#157](https://github.com/JorisJonkers-dev/agent-kit/issues/157)


### Bug Fixes

* **manifest:** pin the agent-session-bootstrap skill at its current content ([#204](https://github.com/JorisJonkers-dev/agent-kit/issues/204)) ([b5f8964](https://github.com/JorisJonkers-dev/agent-kit/commit/b5f8964f4edfee721fceba047305ad9eb738da41))
* **overleaf:** let overleaf writes run without a prompt ([#201](https://github.com/JorisJonkers-dev/agent-kit/issues/201)) ([519b15d](https://github.com/JorisJonkers-dev/agent-kit/commit/519b15da7a46125368e5d6a5e1ceb9620f29e6b9))
* **plugins:** update codenamev/ai-software-architect digest to 3f030c1 ([#149](https://github.com/JorisJonkers-dev/agent-kit/issues/149)) ([c3fa2f6](https://github.com/JorisJonkers-dev/agent-kit/commit/c3fa2f6624f051186738f9cab7b36359948e6dea))
* **plugins:** update lukasniessen/kubernetes-skill digest to 34f93c1 ([#155](https://github.com/JorisJonkers-dev/agent-kit/issues/155)) ([c23c501](https://github.com/JorisJonkers-dev/agent-kit/commit/c23c5010b23cdf2275c4cbff782a9886537c7c6c))

## [4.16.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.15.1...v4.16.0) (2026-10-20)


### Features

* nothing on the install path needs the knowledge base ([#142](https://github.com/JorisJonkers-dev/agent-kit/issues/142)) ([becd2e4](https://github.com/JorisJonkers-dev/agent-kit/commit/becd2e4bb5bb73e700666b1f70a1540d91aa8ff9))


### Bug Fixes

* drop the retired knowledge-api from council, skills and docs ([#147](https://github.com/JorisJonkers-dev/agent-kit/issues/147)) ([987e574](https://github.com/JorisJonkers-dev/agent-kit/commit/987e574fa67b0e7b1e7ba7a10fbb8bc8f59128f3))
* **plugins:** update anthropics/claude-plugins-official digest to ea10b21 ([#144](https://github.com/JorisJonkers-dev/agent-kit/issues/144)) ([dfbd728](https://github.com/JorisJonkers-dev/agent-kit/commit/dfbd728b3dbb8ef1fac5e828705fbb9b0e6f432f))
* **plugins:** update codenamev/ai-software-architect digest to 2e6309a ([#145](https://github.com/JorisJonkers-dev/agent-kit/issues/145)) ([4614177](https://github.com/JorisJonkers-dev/agent-kit/commit/4614177287894da54db8a8b8199dace7d1e12317))

## [4.15.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.15.0...v4.15.1) (2026-10-18)


### Bug Fixes

* **mcp:** register an oauth server on Codex without starting its sign-in ([#140](https://github.com/JorisJonkers-dev/agent-kit/issues/140)) ([626cdcb](https://github.com/JorisJonkers-dev/agent-kit/commit/626cdcb562d38bcdba9fb4a4913e025675a883ce))

## [4.15.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.14.0...v4.15.0) (2026-10-17)


### Features

* **mcp:** an oauth auth kind, and n8n registered by URL on Claude, Codex and Vibe ([#137](https://github.com/JorisJonkers-dev/agent-kit/issues/137)) ([bb59fd0](https://github.com/JorisJonkers-dev/agent-kit/commit/bb59fd0e6ef0676838fc66f56b277aef0f5314a3))
* **mcp:** n8n-mcp runs docs-only from one pinned global binary ([#139](https://github.com/JorisJonkers-dev/agent-kit/issues/139)) ([ee07ebf](https://github.com/JorisJonkers-dev/agent-kit/commit/ee07ebf762d9e3c12b1d27169cf33c70b0a6ee21))

## [4.14.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.13.0...v4.14.0) (2026-10-16)


### Features

* **mcp:** start Playwright from one pinned global binary on every workstation agent ([#134](https://github.com/JorisJonkers-dev/agent-kit/issues/134)) ([6a05de2](https://github.com/JorisJonkers-dev/agent-kit/commit/6a05de2ffcb8c062fb216444145605bfa590dd93))

## [4.13.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.12.1...v4.13.0) (2026-10-16)


### Features

* **mcp:** converge every surface's MCP servers to the registry and the keep file ([#130](https://github.com/JorisJonkers-dev/agent-kit/issues/130)) ([a7b2d56](https://github.com/JorisJonkers-dev/agent-kit/commit/a7b2d56670757319b34022f5213807c4e4f38d96)), closes [#118](https://github.com/JorisJonkers-dev/agent-kit/issues/118)
* **memory:** retire memory-mcp and unregister retired servers from every agent ([#109](https://github.com/JorisJonkers-dev/agent-kit/issues/109)) ([35087ca](https://github.com/JorisJonkers-dev/agent-kit/commit/35087ca61eaf99b6412bc16f295994b3bfa8fd1e))
* **plugins:** converge Claude marketplaces and plugins to the registry and the keep file ([#131](https://github.com/JorisJonkers-dev/agent-kit/issues/131)) ([1a651f2](https://github.com/JorisJonkers-dev/agent-kit/commit/1a651f290209c29ae4ca35a55043826ed7149dc4)), closes [#119](https://github.com/JorisJonkers-dev/agent-kit/issues/119)

## [4.12.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.12.0...v4.12.1) (2026-10-16)


### Bug Fixes

* **plugins:** declare claude-plugins-official under another name ([#107](https://github.com/JorisJonkers-dev/agent-kit/issues/107)) ([62df5eb](https://github.com/JorisJonkers-dev/agent-kit/commit/62df5eba65da969bfc17931af46de3e0ec90e966))

## [4.12.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.11.0...v4.12.0) (2026-10-16)


### Features

* **plugins:** move marketplace pins nightly, held for review when a bump adds code to run ([#105](https://github.com/JorisJonkers-dev/agent-kit/issues/105)) ([df385b7](https://github.com/JorisJonkers-dev/agent-kit/commit/df385b77885c02bd83d6d22b10354ea70bf34c22))

## [4.11.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.10.1...v4.11.0) (2026-10-16)


### Features

* **plugins:** install every plugin at its marketplace's pinned commit ([#103](https://github.com/JorisJonkers-dev/agent-kit/issues/103)) ([71ad9a3](https://github.com/JorisJonkers-dev/agent-kit/commit/71ad9a33f44c5ccc6bf933dd24d1f57cced682d9))

## [4.10.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.10.0...v4.10.1) (2026-10-15)


### Bug Fixes

* **vibe:** let Vibe use your Mistral account, not a key setup exported ([#101](https://github.com/JorisJonkers-dev/agent-kit/issues/101)) ([e0fe6ce](https://github.com/JorisJonkers-dev/agent-kit/commit/e0fe6cebe8f5f07cead3429ed7073ba7b9bb517e))

## [4.10.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.9.0...v4.10.0) (2026-10-15)


### Features

* **vibe:** offer every Mistral chat model, and keep Codex's memory hooks ([#99](https://github.com/JorisJonkers-dev/agent-kit/issues/99)) ([76595c6](https://github.com/JorisJonkers-dev/agent-kit/commit/76595c6629f86f3a01d47ec18c5d3fd4fdeb6b7f))

## [4.9.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.8.0...v4.9.0) (2026-10-15)


### Features

* **memory:** replace the deprecated Hindsight plugin with Coding Agents, recall only ([#97](https://github.com/JorisJonkers-dev/agent-kit/issues/97)) ([e35f869](https://github.com/JorisJonkers-dev/agent-kit/commit/e35f869c770f04f9a4c98792d6b2b9f573a1b1ad))

## [4.8.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.7.3...v4.8.0) (2026-10-15)


### Features

* **private:** seal the private-bank token behind a passphrase ([#95](https://github.com/JorisJonkers-dev/agent-kit/issues/95)) ([761010a](https://github.com/JorisJonkers-dev/agent-kit/commit/761010a61a4063e4fb11ec3d0c4ee892ce503956))

## [4.7.3](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.7.2...v4.7.3) (2026-10-15)


### Bug Fixes

* **keys:** finish the walk-through when sign-out answers with a page ([#93](https://github.com/JorisJonkers-dev/agent-kit/issues/93)) ([fd86ceb](https://github.com/JorisJonkers-dev/agent-kit/commit/fd86cebbeb769427922cbf0a8bed7d5bd08d653d))

## [4.7.2](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.7.1...v4.7.2) (2026-10-15)


### Bug Fixes

* **keys:** trust the system CA bundle, and refuse redirects in the retain hook ([#91](https://github.com/JorisJonkers-dev/agent-kit/issues/91)) ([1067512](https://github.com/JorisJonkers-dev/agent-kit/commit/10675121e2e7a5b1acff3fdddab6a1c7cd0c37da))

## [4.7.1](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.7.0...v4.7.1) (2026-10-15)


### Bug Fixes

* **setup:** fetch the kit bundle by its checksum, past the CDN cache ([#89](https://github.com/JorisJonkers-dev/agent-kit/issues/89)) ([811b7ff](https://github.com/JorisJonkers-dev/agent-kit/commit/811b7ff549a1af2cf53a7a8b2d0d5ef7b31e17c2))

## [4.7.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.6.0...v4.7.0) (2026-10-15)


### Features

* **setup:** walk through the tokens and keys instead of listing them ([#87](https://github.com/JorisJonkers-dev/agent-kit/issues/87)) ([f2f39ab](https://github.com/JorisJonkers-dev/agent-kit/commit/f2f39abf20e87fee7eab269b986506413f2cb057))

## [4.6.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.5.0...v4.6.0) (2026-10-15)


### Features

* **setup:** end every run with one list of everything still to do ([#85](https://github.com/JorisJonkers-dev/agent-kit/issues/85)) ([b33225d](https://github.com/JorisJonkers-dev/agent-kit/commit/b33225d358e30bf2f162a2c67029377e98e8df40))

## [4.5.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.4.0...v4.5.0) (2026-10-15)


### Features

* **vibe:** retain Vibe's turns into the private Hindsight bank ([#83](https://github.com/JorisJonkers-dev/agent-kit/issues/83)) ([e093c3f](https://github.com/JorisJonkers-dev/agent-kit/commit/e093c3f653ff26b0b5918b3e04b9cd0f0b9ab5d6))

## [4.4.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.3.0...v4.4.0) (2026-10-15)


### Features

* **registry:** turn the private Hindsight bank on in Vibe ([#81](https://github.com/JorisJonkers-dev/agent-kit/issues/81)) ([9822998](https://github.com/JorisJonkers-dev/agent-kit/commit/982299833507811325083af8f47ca798bcad53a9))

## [4.3.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.2.0...v4.3.0) (2026-10-15)


### Features

* **registry:** add memory-private, the private Hindsight bank, switched off by default ([#80](https://github.com/JorisJonkers-dev/agent-kit/issues/80)) ([9391974](https://github.com/JorisJonkers-dev/agent-kit/commit/93919749266aeafeb12052973d89ed0ddf1ce8e5))
* **registry:** Hermes reaches Hindsight through the gated shared host ([#78](https://github.com/JorisJonkers-dev/agent-kit/issues/78)) ([1fe7d17](https://github.com/JorisJonkers-dev/agent-kit/commit/1fe7d17622a0838a4ef0e88ab9c8690038c60f28))

## [4.2.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.1.0...v4.2.0) (2026-10-14)


### Features

* **installer:** install and configure Mistral Vibe like the other agents ([#76](https://github.com/JorisJonkers-dev/agent-kit/issues/76)) ([1cff656](https://github.com/JorisJonkers-dev/agent-kit/commit/1cff656e734e2480c8286bca4db2245f08da11f1))
* **installer:** memory-private keeps a directory out of Hindsight auto-retain ([#77](https://github.com/JorisJonkers-dev/agent-kit/issues/77)) ([6731230](https://github.com/JorisJonkers-dev/agent-kit/commit/6731230c338246427a554a61ccaf20eb8d0f75b3))
* **installer:** set up a Claude Code cloud environment with --cloud ([#72](https://github.com/JorisJonkers-dev/agent-kit/issues/72)) ([7ce49b8](https://github.com/JorisJonkers-dev/agent-kit/commit/7ce49b828adf08c873c8ad1fe76fdea9a0d4c7cf))
* **registry:** point memory-api and the hindsight plugin at the shared bank ([#75](https://github.com/JorisJonkers-dev/agent-kit/issues/75)) ([3ea19b6](https://github.com/JorisJonkers-dev/agent-kit/commit/3ea19b68054d3c6c3a1b3b82e03c2ecf34624cee))
* **registry:** point memory-api at hindsight-api.jorisjonkers.dev ([#74](https://github.com/JorisJonkers-dev/agent-kit/issues/74)) ([52c6063](https://github.com/JorisJonkers-dev/agent-kit/commit/52c6063795c4245c3d5b3a0699d38390135499fc))


### Bug Fixes

* **runner:** drop the retired knowledge MCP from the runner image ([#68](https://github.com/JorisJonkers-dev/agent-kit/issues/68)) ([d3bbe38](https://github.com/JorisJonkers-dev/agent-kit/commit/d3bbe384b0caacf1fe3b9fe6a2bade52bf14f29d))

## [4.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.0.0...v4.1.0) (2026-09-21)


### Features

* **ci:** attach installer artifacts to the release on publish ([#66](https://github.com/JorisJonkers-dev/agent-kit/issues/66)) ([05cc561](https://github.com/JorisJonkers-dev/agent-kit/commit/05cc5616cbe712765e1cf7b84cc9568e8a3cfca4))
* **installer:** make setup-workstation.sh a true superset, retire install.sh ([#65](https://github.com/JorisJonkers-dev/agent-kit/issues/65)) ([b7c1496](https://github.com/JorisJonkers-dev/agent-kit/commit/b7c1496349582d9b5387f7444faf24d0e4a5798a))
* **installer:** second Claude Code profile and claude-&lt;profile&gt; launcher ([#58](https://github.com/JorisJonkers-dev/agent-kit/issues/58)) ([ce69717](https://github.com/JorisJonkers-dev/agent-kit/commit/ce697172aacaa9787083fb3df526574be8500af4))
* **registry:** generate a container setup script for the agents image ([#62](https://github.com/JorisJonkers-dev/agent-kit/issues/62)) ([019932f](https://github.com/JorisJonkers-dev/agent-kit/commit/019932f422f23f92d3280f248cf2217760016658))
* **registry:** wire Claude Code, Codex and Hermes to Hindsight + Basic Memory ([#64](https://github.com/JorisJonkers-dev/agent-kit/issues/64)) ([0038b81](https://github.com/JorisJonkers-dev/agent-kit/commit/0038b81b9166668ea0ac59bc70686770cba096b1))

## [4.0.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v3.3.0...v4.0.0) (2026-09-11)


### ⚠ BREAKING CHANGES

* the kit installs no hooks. Automatic KB recall and capture are gone; use the recall skill or the `knowledge.recall` MCP tool explicitly.

### Features

* add the estate tooling registry and a generated workstation setup ([#39](https://github.com/JorisJonkers-dev/agent-kit/issues/39)) ([16d825b](https://github.com/JorisJonkers-dev/agent-kit/commit/16d825b82fcf57a7f922f1dcf8736f5bf416bdc2))
* end the setup run by saying what is still left to do ([#51](https://github.com/JorisJonkers-dev/agent-kit/issues/51)) ([16ab466](https://github.com/JorisJonkers-dev/agent-kit/commit/16ab4662a553b40dc14a7e501ecf6d3d759ad062))
* **mcp:** expose the read-only kubernetes MCP to the workstation ([#55](https://github.com/JorisJonkers-dev/agent-kit/issues/55)) ([e907658](https://github.com/JorisJonkers-dev/agent-kit/commit/e9076586d3547b156c82d43752f834676f339866))
* **mcp:** keep the kubernetes MCP port-forward alive with a launchd agent ([#56](https://github.com/JorisJonkers-dev/agent-kit/issues/56)) ([302a9bb](https://github.com/JorisJonkers-dev/agent-kit/commit/302a9bb5832ae1ec5b9ae2308bbcb6be0ee010c9))
* retire every agent hook from the kit ([#37](https://github.com/JorisJonkers-dev/agent-kit/issues/37)) ([709ef03](https://github.com/JorisJonkers-dev/agent-kit/commit/709ef033fb3ef3d9427de978e28fa37dfd7f2003))


### Bug Fixes

* **installer:** stop install-agents.sh registering the retired context7 ([#57](https://github.com/JorisJonkers-dev/agent-kit/issues/57)) ([ff91401](https://github.com/JorisJonkers-dev/agent-kit/commit/ff91401c9ec6357bdaa6a2b4eb1ae5b74b0e9be6))
* make the registry authoritative and stop silent no-op plugins ([#50](https://github.com/JorisJonkers-dev/agent-kit/issues/50)) ([42867e6](https://github.com/JorisJonkers-dev/agent-kit/commit/42867e689736ea710b5e0e73cdef90ec53624822))
* **manifest:** resync stale sha256 pins for worker.md and install.sh ([#28](https://github.com/JorisJonkers-dev/agent-kit/issues/28)) ([8fade6a](https://github.com/JorisJonkers-dev/agent-kit/commit/8fade6aef9519580be63517ba2fcff5fdd6c1ec9))
* **overleaf:** wire the overleaf MCP into codex + local hermes, fix cookie name ([#53](https://github.com/JorisJonkers-dev/agent-kit/issues/53)) ([9d58f36](https://github.com/JorisJonkers-dev/agent-kit/commit/9d58f36bcf9cf98803883f0196ea4ca97211ad2d))

## [3.3.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v3.2.0...v3.3.0) (2026-07-12)


### Features

* **council:** stall detection with liveness contract, watchdog, and poll-until-green ([#22](https://github.com/JorisJonkers-dev/agent-kit/issues/22)) ([134d014](https://github.com/JorisJonkers-dev/agent-kit/commit/134d014ccc957af0fd146c289541729385b808c3))
* **monitor:** add monitor primitive with poll-until-finalizer and state persistence ([#20](https://github.com/JorisJonkers-dev/agent-kit/issues/20)) ([b1c06c3](https://github.com/JorisJonkers-dev/agent-kit/commit/b1c06c3dbb16aebff8475d1147f858f102cd7a03))

## [3.2.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v3.1.0...v3.2.0) (2026-07-08)


### Features

* deploy-status and prepare-rollback agent skills ([#16](https://github.com/JorisJonkers-dev/agent-kit/issues/16)) ([1749cb2](https://github.com/JorisJonkers-dev/agent-kit/commit/1749cb24c3aed54d1cbc1747db22fde801a364f1))

## [3.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v3.0.0...v3.1.0) (2026-07-07)


### Features

* **runner-runtime:** default git identity to JorisJonkers Agent ([#14](https://github.com/JorisJonkers-dev/agent-kit/issues/14)) ([8092862](https://github.com/JorisJonkers-dev/agent-kit/commit/809286227a788b5f741116ff0738e17d903e3a96))

## [3.0.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v2.0.0...v3.0.0) (2026-07-03)


### ⚠ BREAKING CHANGES

* **hooks:** installs no longer register a UserPromptSubmit recall hook.

### Features

* **council:** live observability — status --watch + tail ([#13](https://github.com/JorisJonkers-dev/agent-kit/issues/13)) ([80a4a88](https://github.com/JorisJonkers-dev/agent-kit/commit/80a4a8885efa1ab199b5113fad25f4f11bf2edd6))
* **council:** native DAG executor for fanout/fleet (--execute) ([#9](https://github.com/JorisJonkers-dev/agent-kit/issues/9)) ([0472566](https://github.com/JorisJonkers-dev/agent-kit/commit/0472566e08ae90f57b5f1eece096c10a7c56f6ce))
* **hooks:** remove the UserPromptSubmit knowledge-recall hook ([#12](https://github.com/JorisJonkers-dev/agent-kit/issues/12)) ([1202b78](https://github.com/JorisJonkers-dev/agent-kit/commit/1202b7845422995dc3bb6401e5a394f7a31b82ac))


### Bug Fixes

* **hooks:** default recall hook to fast mode so it stays under its timeout ([#11](https://github.com/JorisJonkers-dev/agent-kit/issues/11)) ([bedeba2](https://github.com/JorisJonkers-dev/agent-kit/commit/bedeba2a161f9bfa99d7d914651d66ee81f510c1))

## [2.0.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v1.2.0...v2.0.0) (2026-07-03)


### ⚠ BREAKING CHANGES

* **council:** the installed council entrypoint is now 'node ~/.claude/skills/council/council.mjs'; the python3 council.py invocation no longer exists.

### Features

* **council:** TypeScript orchestrator with triage, watchdog, review councils, and cross-CLI surfaces ([#6](https://github.com/JorisJonkers-dev/agent-kit/issues/6)) ([43fde60](https://github.com/JorisJonkers-dev/agent-kit/commit/43fde60dc75127c333729efb7883ae8e3981cd09))

## [1.2.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v1.1.0...v1.2.0) (2026-06-29)


### Features

* apply agent-kit program sweep ([#3](https://github.com/JorisJonkers-dev/agent-kit/issues/3)) ([e0d8533](https://github.com/JorisJonkers-dev/agent-kit/commit/e0d8533edf747de9cb8aa84d7c45aab65cd719bd))

## [1.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v1.0.0...v1.1.0) (2026-06-28)


### Features

* **installer:** add full agents-system installer (install-agents.sh) ([#13](https://github.com/JorisJonkers-dev/agent-kit/issues/13)) ([fbcb27e](https://github.com/JorisJonkers-dev/agent-kit/commit/fbcb27ea9fb0a7fd1dcf864cf913ee2a8ff4ab3e))
* **installer:** auto-wire Claude hooks in install-agents.sh ([#14](https://github.com/JorisJonkers-dev/agent-kit/issues/14)) ([0fc1d0c](https://github.com/JorisJonkers-dev/agent-kit/commit/0fc1d0c32c512614293baccfca76d4e73b1880a9))
* **installer:** register the portable MCP fleet in install-agents.sh ([#15](https://github.com/JorisJonkers-dev/agent-kit/issues/15)) ([9349709](https://github.com/JorisJonkers-dev/agent-kit/commit/93497099954858f8f00b0b0a406d31aab15b0461))
* **runtime:** publish agent kit runtime home bundle ([#1](https://github.com/JorisJonkers-dev/agent-kit/issues/1)) ([f370a23](https://github.com/JorisJonkers-dev/agent-kit/commit/f370a234ffe32d2cdd4ecf8d658a0a3d654a2f28))


### Bug Fixes

* author release-please PR with the GitHub App token ([#16](https://github.com/JorisJonkers-dev/agent-kit/issues/16)) ([6a3a530](https://github.com/JorisJonkers-dev/agent-kit/commit/6a3a530cf1b9d2812c258e4b3b38d389f22ac840))
* **council:** tolerate CLI preamble in run_claude JSON envelope ([#17](https://github.com/JorisJonkers-dev/agent-kit/issues/17)) ([379d35e](https://github.com/JorisJonkers-dev/agent-kit/commit/379d35e8b10abdd4607b48cd3f9fa4fa03508127))
* keep both SSH insteadOf rewrites in runner entrypoint ([#12](https://github.com/JorisJonkers-dev/agent-kit/issues/12)) ([9c29d4b](https://github.com/JorisJonkers-dev/agent-kit/commit/9c29d4bf419c55fa102916c7f857768053cde199))
* quote release.yml publish run step (YAML colon-space error on L33) ([#9](https://github.com/JorisJonkers-dev/agent-kit/issues/9)) ([eb09ef0](https://github.com/JorisJonkers-dev/agent-kit/commit/eb09ef0019fe20b0c761aaa07833030b27dc830e))

## 1.0.0 (2026-06-09)


### Features

* agent-runner runtime packaging (round 4) ([#4](https://github.com/ExtraToast/agent-kit/issues/4)) ([00975d5](https://github.com/ExtraToast/agent-kit/commit/00975d52c6f88ab8535e821994f17f745386417d))
* implement agent kit renderer ([#2](https://github.com/ExtraToast/agent-kit/issues/2)) ([084536c](https://github.com/ExtraToast/agent-kit/commit/084536c6b32a0e6c6f74c9f1b64a2b245493f2b6))
