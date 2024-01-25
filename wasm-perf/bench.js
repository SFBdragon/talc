import init, {bench} from "./pkg/wasm_perf.js";
await init(Deno.readFile('./pkg/wasm_perf_bg.wasm'));
bench();
