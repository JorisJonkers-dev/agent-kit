# Vibe's own answer lists Vibe; the critic agents keep their pinned models

The model gateway takes what Vibe accepts from Vibe itself: it probes
`vibe --model model-gateway-probe -p ping`, a model id reserved for the
gateway, and reads the aliases from the `Available:` line of the refusal.
Before #380 the gateway read `$VIBE_HOME/config.toml`'s `[[models]]` and took
that as the offer. That was wrong in both directions: Vibe's aliases come
from its built-in catalogue too, so a home that deleted or renamed its
entries hid models Vibe still accepts, and an entry Vibe had dropped was
offered for a run that would be refused before it starts. Only Vibe's own
answer measures what Vibe accepts, and it names everything.

We rejected reading the config plus the built-in catalogue list on the
gateway's side (a `vibe config` dump, or a join against a catalogue the
gateway maintains). The catalogue inside Vibe changes with every Vibe
release, away from anything agent-kit pins, and a join the gateway maintains
drifts for exactly the reason the probe was needed: nobody re-runs it after
each upgrade. The probe needs no maintenance and measures the one thing that
matters — what `vibe --model <id>` accepts today. Its cost is one Vibe start
per `models` call, capped (`--max-turns 1 --max-tokens 1 --max-price 0`, so
even a probe that were accepted could spend nothing), and kept for ten
minutes in the cache, keyed by the Vibe binary and the home's `config.toml`.
The probe fails closed: no answer, no Vibe models, never the catalogue
against Vibe's word.

The same change does not touch the `vibe_critic_agents` of
`registry/estate-tooling.yaml`. Each critic agent file
(`registry/generated/vibe/agents/critic-<model>.toml`) pins its model with
its own `[[models]]` entry — the model name, its per-token prices and its
thinking level — and critique's routing names that model as `alias`. A critic
run is `vibe --agent critic-<model>` with the agent's own model, so the pin
holds whatever `config.toml` says, and the prices that hold the critique
ledger and the plan windows to their caps are the entry's own. Repinning
critics onto whatever the probe happens to answer — or letting their offer
follow the home's sync — would move them onto models nobody has judged the
critique pipeline on. Critique keeps its pinned models until the owner moves
one deliberately, with its prices moved in the same change.

## Consequences

- `config.toml`'s `[[models]]` entries no longer decide the gateway's offer;
  they only supply name joins: an accepted alias is joined to the model its
  configuration names, which the catalogue in `defaults.json` may know it by.
- `scripts/vibe-sync-models.py` keeps its role: it writes `config.toml` so
  `vibe --model <alias>` accepts the models the key can use, and so the
  names a signed-in home joins under are fresh.
- The probe id `model-gateway-probe` is reserved. A home that declares it as
  a model would let the probe run a model instead of being refused, so the
  gateway fails closed and names the clash instead of probing.
- A critic model dropped from Vibe still runs: the agent file's `[[models]]`
  entry is a Vibe model definition, not a pointer at the home's list. When
  the underlying model goes away, that is a deliberate repin, not an
  accident of the probe.
