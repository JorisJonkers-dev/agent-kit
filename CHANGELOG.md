# Changelog

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
