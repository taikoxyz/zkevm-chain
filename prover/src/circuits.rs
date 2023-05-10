use crate::circuit_witness::CircuitWitness;
use eth_types::{Address, H256};
use halo2_proofs::halo2curves::bn256::Fr;
use rand::Rng;
use zkevm_circuits::bytecode_circuit::bytecode_unroller::BytecodeCircuit;
use zkevm_circuits::copy_circuit::CopyCircuit;
use zkevm_circuits::evm_circuit::EvmCircuit;
use zkevm_circuits::exp_circuit::ExpCircuit;
use zkevm_circuits::keccak_circuit::keccak_packed_multi::KeccakCircuit;
use zkevm_circuits::pi_circuit2::PiCircuit;
use zkevm_circuits::pi_circuit2::PiTestCircuit;
use zkevm_circuits::state_circuit::StateCircuit;
// use zkevm_circuits::super_circuit::SuperCircuit;
use zkevm_circuits::evm_circuit::witness::Taiko;
use zkevm_circuits::tx_circuit::TxCircuit;
use zkevm_circuits::util::SubCircuit;
use zkevm_common::prover::ProofRequestOptions;

/// Returns a instance of the `SuperCircuit`.
// pub fn gen_super_circuit<
//     const MAX_TXS: usize,
//     const MAX_CALLDATA: usize,
//     const MAX_RWS: usize,
//     RNG: Rng,
// >(
//     witness: &CircuitWitness,
//     mut _rng: RNG,
// ) -> Result<SuperCircuit<Fr, MAX_TXS, MAX_CALLDATA, MAX_RWS>, String> {
//     let block = witness.evm_witness();

//     let evm_circuit = EvmCircuit::new_from_block(&block);
//     let state_circuit = StateCircuit::new_from_block(&block);
//     let tx_circuit = TxCircuit::new_from_block(&block);
//     let pi_circuit = PiCircuit::new_from_block(&block);
//     let bytecode_circuit = BytecodeCircuit::new_from_block(&block);
//     let copy_circuit = CopyCircuit::new_from_block(&block);
//     let exp_circuit = ExpCircuit::new_from_block(&block);
//     let keccak_circuit = KeccakCircuit::new_from_block(&block);
//     let circuit = SuperCircuit::<_, MAX_TXS, MAX_CALLDATA, MAX_RWS> {
//         evm_circuit,
//         state_circuit,
//         tx_circuit,
//         pi_circuit,
//         bytecode_circuit,
//         copy_circuit,
//         exp_circuit,
//         keccak_circuit,
//     };

//     Ok(circuit)
// }

fn parse_hash(input: &str) -> H256 {
    H256::from_slice(&hex::decode(input).expect("parse_hash"))
}

fn parse_address(input: &String) -> Address {
    eth_types::Address::from_slice(&hex::decode(input).expect("parse_address"))
}

fn as_taiko_witness(task_options: &ProofRequestOptions) -> Taiko {
    Taiko {
        l1_signal_service: parse_address(&task_options.l1_signal_service),
        l2_signal_service: parse_address(&task_options.l2_signal_service),
        l2_contract: parse_address(&task_options.l2_contract),
        meta_hash: parse_hash(&task_options.meta_hash),
        signal_root: parse_hash(&task_options.signal_root),
        graffiti: parse_hash(&task_options.graffiti),
        prover: parse_address(&task_options.prover),
        parent_gas_used: task_options.parent_gas_used,
    }
}

/// Returns a instance of the `PiTestCircuit`.
pub fn gen_pi_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    task_options: &ProofRequestOptions,
    mut _rng: RNG,
) -> Result<PiTestCircuit<Fr>, String> {
    let block = witness.evm_witness();
    let taiko = as_taiko_witness(task_options);
    let circuit = PiTestCircuit::<Fr>(PiCircuit::new_from_block_with_extra(&block, &taiko));

    Ok(circuit)
}

/// Returns a instance of the `EvmCircuit`.
pub fn gen_evm_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<EvmCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(EvmCircuit::new_from_block(&block))
}

/// Returns a instance of the `StateCircuit`.
pub fn gen_state_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<StateCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(StateCircuit::new_from_block(&block))
}

/// Returns a instance of the `TxCircuit`.
pub fn gen_tx_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<TxCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(TxCircuit::new_from_block(&block))
}

/// Returns a instance of the `BytecodeCircuit`.
pub fn gen_bytecode_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<BytecodeCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(BytecodeCircuit::new_from_block(&block))
}

/// Returns a instance of the `CopyCircuit`.
pub fn gen_copy_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<CopyCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(CopyCircuit::new_from_block(&block))
}

/// Returns a instance of the `ExpCircuit`.
pub fn gen_exp_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<ExpCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(ExpCircuit::new_from_block(&block))
}

/// Returns a instance of the `KeccakCircuit`.
pub fn gen_keccak_circuit<
    const MAX_TXS: usize,
    const MAX_CALLDATA: usize,
    const MAX_RWS: usize,
    RNG: Rng,
>(
    witness: &CircuitWitness,
    mut _rng: RNG,
) -> Result<KeccakCircuit<Fr>, String> {
    let block = witness.evm_witness();
    Ok(KeccakCircuit::new_from_block(&block))
}
