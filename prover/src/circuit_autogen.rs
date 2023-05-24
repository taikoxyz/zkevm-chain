#[macro_export]
macro_rules! match_circuit_params {
    ($gas_used:expr, $on_match:expr, $on_error:expr) => {
        match $gas_used {
            0..=63000 => {
                const CIRCUIT_CONFIG: CircuitConfig = CircuitConfig {
                    block_gas_limit: 63000,
                    max_txs: 3,
                    max_calldata: 10500,
                    max_bytecode: 24634,
                    max_rws: 476052,
                    min_k: 20,
                    pad_to: 476052,
                    min_k_aggregation: 26,
                    keccak_padding: 336000,
                };
                $on_match
            }
            63001..=300000 => {
                const CIRCUIT_CONFIG: CircuitConfig = CircuitConfig {
                    block_gas_limit: 300000,
                    max_txs: 14,
                    max_calldata: 69750,
                    max_bytecode: 139500,
                    max_rws: 3161966,
                    min_k: 23,
                    pad_to: 3161966,
                    min_k_aggregation: 26,
                    keccak_padding: 1600000,
                };
                $on_match
            }

            _ => $on_error,
        }
    };
}

#[macro_export]
macro_rules! match_circuit_params_txs {
    ($txs:expr, $on_match:expr, $on_error:expr) => {
        match $txs {
            0..=80 => {
                const CIRCUIT_CONFIG: CircuitConfig = CircuitConfig {
                    block_gas_limit: 6000000,
                    max_txs: 10,
                    max_calldata: 131072,
                    max_bytecode: 131072,
                    max_rws: 3161966,
                    min_k: 19,
                    pad_to: 2097152,
                    min_k_aggregation: 26,
                    keccak_padding: 336000,
                };
                $on_match
            }

            _ => $on_error,
        }
    };
}
