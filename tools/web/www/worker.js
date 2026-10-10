// A pool worker: instantiate the same module on the shared memory, report ready, then run std threads' jobs forever (fnweb_worker_main never
// returns; it parks on a futex between jobs). See tools/web/fn-web-rt/src/pool.rs.
import init, * as wasm from './pkg/__FNWEB_LIB__.js';

self.onmessage = async (e) => {
    const d = e.data || {};
    if (d.fnweb !== 'start') return;
    try {
        await init({ module_or_path: d.module, memory: d.memory, thread_stack_size: d.stack });
    } catch (err) {
        self.postMessage({ fnweb: 'error', text: 'init failed: ' + (err && err.stack || err) });
        return;
    }
    self.postMessage({ fnweb: 'ready', index: d.index });
    wasm.fnweb_worker_main(d.index);
};
