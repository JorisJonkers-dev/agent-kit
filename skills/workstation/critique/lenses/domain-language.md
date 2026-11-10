# Lens: domain language

Check that the words the code and the documents use for the domain mean what
the glossary says, and that every context names its relations to the others.
`CONTEXT.md` is the glossary; a term it defines is the declared meaning.

- `term-two-meanings`: One word that means two different things in two
  places, in the docs or the code. Quote both uses, and say what each one
  would change in the code.
- `two-names`: Two names for one concept, in the docs or the code, such as
  `account` in one place and `customer` in another for the same thing. Quote
  both names.
- `term-unglossed`: A domain word the code or the records use for a rule or a
  thing, that `CONTEXT.md` does not define and that a reader would need
  defined to know what it means.
- `relation-unnamed`: Two contexts that share a concept or a call with no
  named relationship: a shared kernel, customer/supplier, conformist, anti-
  corruption layer or published language. Quote the crossing.
- `shared-store`: One store (a table, a bucket, a topic) written by two
  contexts. Name both writers, with a quote of each write.

A term is a finding only when the two readings lead to different code, or the
unnamed relation has a visible consequence.
