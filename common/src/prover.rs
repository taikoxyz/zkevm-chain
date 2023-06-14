use eth_types::{Bytes, U256, Address, H256};
use serde::{Deserialize, Serialize};
use zkevm_circuits::witness::ProtocolInstance;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProofResult {
    /// The halo2 transcript
    pub proof: Bytes,
    /// Public inputs for the proof
    pub instance: Vec<U256>,
    /// k of circuit parameters
    pub k: u8,
    /// Randomness used
    pub randomness: Bytes,
    /// Circuit name / identifier
    pub label: String,
    /// Auxiliary
    pub aux: ProofResultInstrumentation,
}

impl std::fmt::Debug for ProofResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proof")
            .field("proof", &format!("{}", &self.proof))
            .field("instance", &self.instance)
            .field("k", &self.k)
            .field("randomness", &format!("{}", &self.randomness))
            .field("aux", &format!("{:#?}", self.aux))
            .finish()
    }
}

/// Timing information in milliseconds.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct ProofResultInstrumentation {
    /// keygen_vk
    pub vk: u32,
    /// keygen_pk
    pub pk: u32,
    /// create_proof
    pub proof: u32,
    /// verify_proof
    pub verify: u32,
    /// MockProver.verify_par
    pub mock: u32,
    /// Circuit::new
    pub circuit: u32,
    /// RootCircuit::compile
    pub protocol: u32,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Proofs {
    /// Circuit configuration used
    pub config: CircuitConfig,
    // Proof result for circuit
    pub circuit: ProofResult,
    /// Aggregation proof for circuit, if requested
    pub aggregation: ProofResult,
    /// Gas used. Determines the upper ceiling for circuit parameters
    pub gas: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
// request extra instance corresponding to ProtocolInstance
pub struct RequestExtraInstance {
    /// l1 signal service address
    pub l1_signal_service: String,
    /// l2 signal service address
    pub l2_signal_service: String,
    /// l2 contract address
    pub l2_contract: String,
    /// meta hash
    pub meta_hash: String,
    /// block hash value
    pub block_hash: String,
    /// the parent block hash
    pub parent_hash: String,
    /// signal root
    pub signal_root: String,
    /// extra message
    pub graffiti: String,
    /// Prover address
    pub prover: String,
    /// gas used
    pub gas_used: u32,
    /// parent gas used
    pub parent_gas_used: u32,
    /// blockMaxGasLimit
    pub block_max_gas_limit: u64,
    /// maxTransactionsPerBlock
    pub max_transactions_per_block: u64,
    /// maxBytesPerTxList
    pub max_bytes_per_tx_list: u64,
}

fn parse_hash(input: &str) -> H256 {
    H256::from_slice(&hex::decode(input).expect("parse_hash"))
}

fn parse_address(input: &str) -> Address {
    Address::from_slice(&hex::decode(input).expect("parse_address"))
}

impl From<RequestExtraInstance> for ProtocolInstance {
    fn from(instance: RequestExtraInstance) -> Self {
        ProtocolInstance {
            l1_signal_service: parse_address(&instance.l1_signal_service),
            l2_signal_service: parse_address(&instance.l2_signal_service),
            l2_contract: parse_address(&instance.l2_contract),
            meta_hash:   parse_hash(&instance.meta_hash),
            block_hash:  parse_hash(&instance.block_hash),
            parent_hash: parse_hash(&instance.parent_hash),
            signal_root: parse_hash(&instance.signal_root),
            graffiti: parse_hash(&instance.graffiti),
            prover: parse_address(&instance.prover),
            gas_used: instance.gas_used,
            parent_gas_used: instance.parent_gas_used,
            block_max_gas_limit: instance.block_max_gas_limit,
            max_transactions_per_block: instance.max_transactions_per_block,
            max_bytes_per_tx_list: instance.max_bytes_per_tx_list,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProofRequestOptions {
    /// The name of the circuit.
    /// "super", "pi"
    pub circuit: String,
    /// the block number
    pub block: u64,
    /// the l2 rpc url
    pub rpc: String,
    /// the protocol instance data
    pub protocol_instance: RequestExtraInstance,
    /// retry proof computation if error
    pub retry: bool,
    /// Parameters file or directory to use.
    /// Otherwise generates them on the fly.
    pub param: Option<String>,
    /// Only use MockProver if true.
    #[serde(default = "default_bool")]
    pub mock: bool,
    /// Additionaly aggregates the circuit proof if true
    #[serde(default = "default_bool")]
    pub aggregate: bool,
    /// Runs the MockProver if proofing fails.
    #[serde(default = "default_bool")]
    pub mock_feedback: bool,
    /// Verifies the proof after computation.
    #[serde(default = "default_bool")]
    pub verify_proof: bool,
}

impl PartialEq for ProofRequestOptions {
    fn eq(&self, other: &Self) -> bool {
        self.block == other.block
            && self.protocol_instance.prover == other.protocol_instance.prover
            && self.protocol_instance.l1_signal_service == other.protocol_instance.l1_signal_service
            && self.protocol_instance.l2_signal_service == other.protocol_instance.l2_signal_service
            && self.protocol_instance.l2_contract == other.protocol_instance.l2_contract
            && self.protocol_instance.block_hash == other.protocol_instance.block_hash
            && self.protocol_instance.parent_hash == other.protocol_instance.parent_hash
            && self.protocol_instance.meta_hash == other.protocol_instance.meta_hash
            && self.protocol_instance.signal_root == other.protocol_instance.signal_root
            && self.protocol_instance.graffiti == other.protocol_instance.graffiti
            && self.protocol_instance.gas_used == other.protocol_instance.gas_used
            && self.protocol_instance.parent_gas_used == other.protocol_instance.parent_gas_used
            && self.protocol_instance.max_bytes_per_tx_list == other.protocol_instance.max_bytes_per_tx_list
            && self.protocol_instance.block_max_gas_limit == other.protocol_instance.block_max_gas_limit
            && self.protocol_instance.max_transactions_per_block == other.protocol_instance.max_transactions_per_block
            && self.rpc == other.rpc
            && self.param == other.param
            && self.circuit == other.circuit
            && self.mock == other.mock
            && self.aggregate == other.aggregate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRequest {
    pub options: ProofRequestOptions,
    pub result: Option<Result<Proofs, String>>,
    /// A counter to keep track of changes of the `result` field
    pub edition: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeInformation {
    pub id: String,
    pub tasks: Vec<ProofRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: String,
    /// The current active task this instance wants to obtain or is working on.
    pub task: Option<ProofRequestOptions>,
    /// `true` if this instance started working on `task`
    pub obtained: bool,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct CircuitConfig {
    pub block_gas_limit: usize,
    pub max_txs: usize,
    pub max_calldata: usize,
    pub max_bytecode: usize,
    pub max_rws: usize,
    pub max_copy_rows: usize,
    pub max_exp_steps: usize,
    pub min_k: usize,
    pub pad_to: usize,
    pub min_k_aggregation: usize,
    pub keccak_padding: usize,
}

fn default_bool() -> bool {
    false
}
