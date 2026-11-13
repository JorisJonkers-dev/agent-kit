// Preloaded into the node runner (node --import): `new Date()` and Date.now()
// answer FIXED_NOW, so the page's "updated ..." stamp is the same every run.
import { performance } from 'node:perf_hooks'

const RealDate = Date
const fixed = RealDate.parse(process.env.FIXED_NOW ?? '2026-11-03T12:00:00Z')

globalThis.Date = class extends RealDate {
  constructor(...args) {
    if (args.length === 0) super(fixed)
    else super(...args)
  }

  static now() {
    return fixed
  }
}

// The monotonic reading is fixed too, so a run's duration (and the moment it adds to a reset it
// names) is the same every time.
performance.now = () => 0
