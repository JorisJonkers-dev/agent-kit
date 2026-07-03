import { constants } from 'node:fs';
import { mkdir, open, rename } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import { assertPathSegment } from './artifact-codec.js';
export class AtomicWriter {
    root;
    tempId;
    constructor(root, tempId) {
        this.root = root;
        this.tempId = tempId;
    }
    async executeJsonPlan(plan) {
        await this.writeBytes(join(this.root, plan.finalPath), join(this.root, plan.tempPath), plan.bytes);
    }
    async writeJson(path, value) {
        const id = this.tempId();
        assertPathSegment('tempId', id);
        await this.writeBytes(path, join(dirname(path), `.${basename(path)}.${id}.tmp`), `${JSON.stringify(value, null, 2)}\n`);
    }
    async writeBytes(finalPath, tempPath, bytes) {
        await mkdir(dirname(finalPath), { recursive: true });
        const handle = await open(tempPath, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY);
        try {
            await handle.write(bytes, 0, 'utf8');
            await handle.sync();
        }
        finally {
            await handle.close();
        }
        await rename(tempPath, finalPath);
        await syncDirectory(dirname(finalPath));
    }
}
export async function syncFile(path) {
    const handle = await open(path, constants.O_RDONLY);
    try {
        await handle.sync();
    }
    finally {
        await handle.close();
    }
}
async function syncDirectory(path) {
    const handle = await open(path, constants.O_RDONLY);
    try {
        await handle.sync();
    }
    finally {
        await handle.close();
    }
}
