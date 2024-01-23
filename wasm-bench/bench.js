import init, {bench} from "./pkg/wasm_alloc_bench.js";
await init(Deno.readFile('./pkg/wasm_alloc_bench_bg.wasm'));
bench();
