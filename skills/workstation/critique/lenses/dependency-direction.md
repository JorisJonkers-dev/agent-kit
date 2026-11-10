# Lens: dependency direction

Check that dependencies point inward, from outer layers (adapters, the
application's entry points, frameworks) to inner ones (the domain), and never
the other way. A layer list, an architecture test or an ADR that names the
layers is the declared rule; without one, a finding needs a visible
consequence (see the design-evidence rules).

- `inward-import`: Find an import from an inner layer into an outer one, such
  as the domain importing an adapter, a database driver or an HTTP client.
  Cite the importing line. It is a finding only when a declared layer rule
  says the imported layer is the inner one.
- `port-in-adapter`: Check that a port (the interface the application
  depends on) is declared beside the code that uses it, not in an adapter,
  and not named for a technology (`PostgresRepository`, `SqsPublisher`).
- `infrastructure-type-in-domain`: Look for database rows, HTTP request or
  response types, or SDK types in the signature of a domain function, type or
  method. Cite the signature.
- `rule-in-adapter`: Look for a business rule (a limit, a state change, a
  calculation the domain should own) written inside an adapter, a handler or
  a controller. Quote the rule and the line that holds it.
