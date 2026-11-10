# Lens: coupling

Check how much each module must know about the others to change. A module
that depends on another's internals, or a decision spread across several
modules, makes every change touch them all.

- `internals-dependence`: A module that depends on another's internals: its
  file format, its table layout, its error text, or a private helper. Quote
  the dependence and name the owner of the internal.
- `decision-spread`: One design decision (a unit, a default, an ordering, a
  status value) encoded in several modules, so that changing it means editing
  each one. Name each place with its line.
- `unexplained-co-change`: A pair of modules that change together in the
  facts with no shared knowledge that explains it. The co-change count is a
  fact from `facts/`; cite it. Co-change that a shared decision explains is not
  a finding.
- `pass-through`: A method that only forwards its arguments to another method
  with the same signature and adds nothing. Quote the method.
- `wide-interface`: An interface that exposes as many operations as its
  implementation has, so callers depend on all of it. Count the operations
  and the implementation's public members, and quote both.

A verdict word such as "tightly coupled" needs a fact or a quoted decision
next to it. Without one, state the number instead.
