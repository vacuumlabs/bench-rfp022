#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let i: sp1_bridge_lib::Input = sp1_zkvm::io::read();
    let j = sp1_bridge_lib::release(&i);
    sp1_zkvm::io::commit_slice(&j);
}
