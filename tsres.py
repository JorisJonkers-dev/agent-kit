import re
def fix(p, f):
    s=open(p).read(); s=f(s); assert '<<<<<<<' not in s, p; open(p,'w').write(s)
def rec(s):
    s,n=re.subn(r"<<<<<<< HEAD\n(import type \{ TicketCheckStatus \}.*?\n)import \{ countVerdicts.*?\n=======\n(import \{\n.*?\} from '../domain/verdicts.js'\n)>>>>>>> origin/impl/189-review-feature\n", r"\1\2", s, flags=re.S); assert n==1
    s,n=re.subn(r"<<<<<<< HEAD\n(.*?)=======\n(.*?)>>>>>>> origin/impl/189-review-feature\n", r"\1\2", s, flags=re.S); assert n==1
    return s
def ver(s):
    s,n=re.subn(r"<<<<<<< HEAD\n(.*?)=======\n(.*?)>>>>>>> origin/impl/189-review-feature\n", r"\1\2", s, flags=re.S); assert n==1
    return s
fix('application/record.ts',rec); fix('domain/verdicts.ts',ver)
