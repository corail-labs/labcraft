//! The function of 1,000 nodes and 2,000 wires that `bounds` draws within 1 s (scenario 6), and that `cargo xtask frames`
//! opens in the viewer to measure 60 frames/s by hand (`docs/architecture.md`, 11.6). Shared by the two, through `#[path]`
//! in `xtask`.

/// The name of its file.
pub const FILE: &str = "f.lcf";

/// `f`: each node adds the two before it, a wire from each.
pub fn nodes_1000_wires_2000() -> String {
    let lines: String = (2..1000).map(|k| format!("    v{k} = core.add(v{}, v{})\n", k - 1, k - 2)).collect();
    format!("labcraft 1\n\nuse core\n\nfn f(x: f64) -> () {{\n    v0 = core.neg(x)\n    v1 = core.add(v0, x)\n{lines}}}\n")
}
