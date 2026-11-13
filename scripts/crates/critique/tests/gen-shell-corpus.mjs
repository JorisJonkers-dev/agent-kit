#!/usr/bin/env node
// Writes the corpus the Rust test `shell_corpus` replays: generated shell command lines, and what the node runner's
// `domain/shell-command.ts` reads from them (the simple commands, and the commits, pushes, merges and PR creations).
//
//   node scripts/crates/critique/tests/gen-shell-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '600', SEED = '17', OUT = join(here, 'fixtures', 'shell-commands.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-shell-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { gatedInvocations, simpleCommands }\n`)
const { gatedInvocations, simpleCommands } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const MSG = () => pick('-m "fix: it\'s done"', "-m 'a (b) c'", '-m "$(cat <<\'EOF\'\nline (one)\nit\'s\nEOF\n)"', '-mmsg', '--message=x', '-F msg.txt', '-S', '-Skey', '-am x', '-sm x', '--author "A <a@b>"', '-c HEAD', '--fixup HEAD~1', '--dry-run', '--', '-- a.ts', 'a.ts', '-i a.ts', '--only', '-p', '-v', '--amend --no-edit')
const PUSH = () => pick('', 'origin', 'origin main', 'origin HEAD', 'origin @', 'origin HEAD:refs/heads/main', 'origin +feat:feat', 'origin :old', 'origin refs/tags/v1', 'origin v1:refs/tags/v1', '--tags', 'origin --tags', '--all', '--mirror', '--branches', '-d origin x', '--delete origin x', '-u origin feat/x', '--force-with-lease origin feat/x', '--dry-run origin main', '-n', '--repo r main x', '-o ci.skip origin main', 'origin -- main', 'origin a:', 'origin refs/heads/main')
const MERGE = () => pick('', 'main', 'origin/main feat', '--no-ff main', '--ff main', '--ff-only main', '--squash main', '--no-commit main', '--continue', '--abort', '-m "m (x)" main', '-mmsg main', '-s ours main', '-Xtheirs main', '-sX ours main', '--strategy=ours main', '--strategy ours main', '--commit --squash main', '--squash --commit main', '-- main')
const PULL = () => pick('', 'origin', 'origin main', 'origin +main:main refs/heads/dev', '--rebase', '-r', '--rebase=false origin main', '--no-rebase', '--ff-only', '--depth 1 origin main', '-j4 origin main', '-j 4 origin main', '--squash origin main', '--no-ff origin main')
const DIR = () => pick('sub', '../other', '/abs/repo', '~', '~/proj', '-', '$HOME/x', '', '.', 'a b')
const GIT_PRE = () => pick('', '', '-C ' + DIR(), '-c user.name=x', '--git-dir=.git', '--no-pager', '-C sub -C ../x', '--work-tree w')
const simple = () =>
  pick(
    () => `git ${GIT_PRE()} commit ${some(MSG, 2).join(' ')}`,
    () => `git ${GIT_PRE()} push ${PUSH()}`,
    () => `git ${GIT_PRE()} merge ${MERGE()}`,
    () => `git ${GIT_PRE()} pull ${PULL()}`,
    () => `git ${pick('add .', 'add -A', 'stage x', 'rm x', 'checkout main', 'switch -c x', 'rebase main', 'reset --hard', 'stash', 'status', 'log --oneline', 'fetch', 'am x.patch', 'cherry-pick abc', 'revert HEAD', 'mv a b', 'restore x', 'update-index --refresh')}`,
    () => `cd ${DIR()}`,
    () => 'cd',
    () => `gh pr create ${pick('', '--fill', '--head feat/x', '-H feat/y', '--head=feat/z', '--title "a (b)" --body "$(cat <<EOF\nbody\nEOF\n)"', '--draft --base main')}`,
    () => `gh ${pick('pr view', 'pr list', 'issue create', '--repo o/r pr create', 'pr --x create')}`,
    () => `${pick('env FOO=1', 'FOO=1 BAR=2', 'command', 'exec', 'nohup', 'time -p', 'builtin', 'env -i A=b', 'x=1')} git ${pick('commit -m x', 'push')}`,
    () => pick('echo "git commit -m x"', "echo 'git push'", 'echo `git push`', 'echo $(git commit -m x)', '# git commit -m x', 'true # git push', 'echo git\\ commit', 'ls > out.txt 2>&1', 'cat <<-EOF\n\tgit commit -m x\n\tEOF', 'cat <<"END"\ngit push\nEND', 'cat <<<word', 'npm test', '2>/dev/null git push', 'git commit -m x >log 2>&1', 'echo "unterminated', "echo 'unterminated", 'echo \\', 'echo "a\\', 'x=$(echo "(" \')\' `y`)', 'echo $(( 1 + 2 ))', 'echo ${A:-git push}', 'x=$(echo "a\\"b $(date) `y`" ) && git push', 'y=$(cat << \'EOF\'\nit\'s (x)\nEOF\n) git commit -m "$y"', 'z=$(echo \\)) && git push', 'cat << EOF\ngit push\nEOF'),
  )()
const SEP = () => pick(' && ', ' || ', '; ', ' | ', ' & ', '\n', ' ; ', ' (', ') ', '\r\n')
const line = () => some(simple, 4).reduce((text, part, i) => (i === 0 ? part : text + SEP() + part), '')
const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const text = line()
  const cwd = pick('/home/u/repo', '/home/u/repo/', '/home/u/../u/repo', '/')
  const home = pick('/home/u', undefined, '/root')
  cases.push({ cwd, home: home ?? null, line: text, expected: { commands: simpleCommands(text), gated: gatedInvocations(text, cwd, home) } })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases written to ${OUT}`)
