#[macro_export]
macro_rules! match_circuit_params {
    ($gas_used:expr, $on_match:expr, $on_error:expr) => {
        match $gas_used {
            0..=8000000 => {
                const CIRCUIT_CONFIG: CircuitConfig = CircuitConfig {
                    block_gas_limit: 800000,
                    max_txs: 14,
                    max_calldata: 69750,
                    max_bytecode: 139500,
                    max_rws: 50000,
                    max_copy_rows: 50000,
                    max_exp_steps: 27900,
                    min_k: 19,
                    pad_to: 0,
                    min_k_aggregation: 22,
                    keccak_padding: 500000,
                };
                $on_match
            }

            _ => $on_error,
        }
    };
}
