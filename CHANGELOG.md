# Changelog

## [4.1.0](https://github.com/JorisJonkers-dev/agent-kit/compare/v4.0.0...v4.1.0) (2026-09-15)


### Features

* **installer:** second Claude Code profile and claude-&lt;profile&gt; launcher ([#58](https://github.com/JorisJonkers-dev/agent-kit/issues/58)) ([ce69717](https://github.com/JorisJonkers-dev/agent-kit/commit/ce697172aacaa9787083fb3df526574be8500af4))

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
