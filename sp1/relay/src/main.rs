#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let i: sp1_bridge_lib::RelayIn = sp1_zkvm::io::read();
    let all: Vec<u8> = sp1_zkvm::io::read_vec();
    let j = sp1_bridge_lib::relay(&i, &all);
    sp1_zkvm::io::commit_slice(&j);
}
