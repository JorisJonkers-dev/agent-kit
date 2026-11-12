# Changelog

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
