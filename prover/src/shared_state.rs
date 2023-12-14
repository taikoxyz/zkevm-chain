use crate::circuit_witness::CircuitWitness;
use crate::circuits::*;
use crate::utils::collect_instance_hex;
use crate::utils::fixed_rng;
use crate::utils::gen_proof;
use crate::Fr;
use crate::G1Affine;
use crate::ProverKey;
use crate::ProverParams;

#[cfg(feature = "evm-verifier")]
mod evm_verifier_helper {
    pub use circuit_benchmarks::taiko_super_circuit::{evm_verify, gen_verifier};
    pub use snark_verifier::loader::evm;
    pub use zkevm_circuits::root_circuit::taiko_aggregation::AccumulationSchemeType;
    pub use zkevm_circuits::root_circuit::Config;
}

use halo2_proofs::dev::MockProver;
use halo2_proofs::plonk::Circuit;
use halo2_proofs::plonk::{keygen_pk, keygen_vk};
use halo2_proofs::poly::commitment::Params;
use halo2_proofs::SerdeFormat;
use hyper::Uri;
use rand::{thread_rng, Rng};
use snark_verifier::system::halo2::transcript::evm::EvmTranscript;
use snark_verifier_sdk::evm::gen_evm_proof_gwc;
use snark_verifier_sdk::halo2::gen_snark_gwc;
use snark_verifier_sdk::CircuitExt;
use snark_verifier_sdk::GWC;
use std::collections::HashMap;
use std::fmt::Write;
use std::fs::File;
use std::io::Write as IoWrite;
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use zkevm_circuits::root_circuit::TaikoAggregationCircuit;
use zkevm_circuits::util::SubCircuit;
use zkevm_common::json_rpc::jsonrpc_request_client;
use zkevm_common::prover::*;

fn get_param_path(path: &String, k: usize) -> PathBuf {
    // try to automatically choose a file if the path is a folder.
    if Path::new(path).is_dir() {
        Path::new(path).join(format!("kzg_bn254_{k}.srs"))
    } else {
        Path::new(path).to_path_buf()
    }
}

fn get_or_gen_param(task_options: &ProofRequestOptions, k: usize) -> (Arc<ProverParams>, String) {
    match &task_options.param {
        Some(v) => {
            let _cur = std::env::current_dir().unwrap();
            let path = get_param_path(v, k);
            let file = File::open(&path).expect(&format!("couldn't open path {:?}", path));
            let params = Arc::new(
                ProverParams::read(&mut std::io::BufReader::new(file))
                    .expect("Failed to read params"),
            );

            (params, path.to_str().unwrap().into())
        }
        None => {
            let param = ProverParams::setup(k as u32, fixed_rng());
            if std::env::var("PROVERD_DUMP").is_ok() {
                param
                    .write_custom(
                        &mut File::create(format!("params-{k}")).unwrap(),
                        SerdeFormat::RawBytesUnchecked,
                    )
                    .unwrap();
            }
            let param = Arc::new(param);
            (param, format!("{k}"))
        }
    }
}

async fn compute_proof<C: Circuit<Fr> + Clone + SubCircuit<Fr> + CircuitExt<Fr>>(
    shared_state: &SharedState,
    task_options: &ProofRequestOptions,
    circuit_config: CircuitConfig,
    circuit: C,
) -> Result<(CircuitConfig, ProofResult, ProofResult), String> {
    log::info!("Using circuit parameters: {:#?}", circuit_config);

    let mut circuit_proof = ProofResult {
        label: format!(
            "{}-{}",
            task_options.circuit, circuit_config.block_gas_limit
        ),
        ..Default::default()
    };
    let mut aggregation_proof = ProofResult {
        label: format!(
            "{}-{}-a",
            task_options.circuit, circuit_config.block_gas_limit
        ),
        ..Default::default()
    };

    if task_options.mock {
        // only run the mock prover
        let time_started = Instant::now();
        circuit_proof.k = circuit_config.min_k as u8;
        circuit_proof.instance = collect_instance_hex(&circuit.instance());
        let prover = MockProver::run(circuit_config.min_k as u32, &circuit, circuit.instance())
            .expect("MockProver::run");
        prover.verify_par().expect("MockProver::verify_par");
        circuit_proof.aux.mock = Instant::now().duration_since(time_started).as_millis() as u32;
    } else {
        let universe_k = circuit_config.min_k.max(circuit_config.min_k_aggregation);
        let (base_param, param_path) = get_or_gen_param(task_options, universe_k);
        let mut aggregation_param = (*base_param).clone();
        let mut circuit_param = aggregation_param.clone();
        if circuit_param.k() as usize > circuit_config.min_k {
            circuit_param.downsize(circuit_config.min_k as u32);
            circuit_proof.k = circuit_param.k() as u8;
        }
        circuit_proof.k = circuit_param.k() as u8;
        // generate and cache the prover key
        let pk = {
            let cache_key = format!(
                "{}{}{:?}",
                &task_options.circuit, &param_path, &circuit_config
            );
            shared_state
                .gen_pk(
                    &cache_key,
                    &Arc::new(circuit_param.clone()),
                    &circuit,
                    &mut circuit_proof.aux,
                )
                .await
                .map_err(|e| e.to_string())?
        };

        let circuit_instance = circuit.instance();
        circuit_proof.instance = collect_instance_hex(&circuit_instance);
        if task_options.aggregate {
            let snark = gen_snark_gwc(&circuit_param, &pk, circuit, None::<&str>);
            circuit_proof.proof = snark.proof.clone().into();
            if std::env::var("PROVERD_DUMP").is_ok() {
                File::create(format!(
                    "proof-{}-{:?}",
                    task_options.circuit, &circuit_config
                ))
                .unwrap()
                .write_all(&snark.proof)
                .unwrap();
            }

            if aggregation_param.k() as usize > circuit_config.min_k_aggregation {
                aggregation_param.downsize(circuit_config.min_k_aggregation as u32);
                aggregation_proof.k = aggregation_param.k() as u8;
            }
            let (agg_params, agg_param_path) = (aggregation_param, param_path.clone());
            aggregation_proof.k = agg_params.k() as u8;
            let agg_circuit = {
                let time_started = Instant::now();
                let v = TaikoAggregationCircuit::<GWC>::new(&agg_params, [snark]).unwrap();
                aggregation_proof.aux.circuit =
                    Instant::now().duration_since(time_started).as_millis() as u32;
                v
            };

            let agg_pk = {
                let cache_key = format!(
                    "{}-agg-{}{:?}",
                    &task_options.circuit, &agg_param_path, &circuit_config
                );
                shared_state
                    .gen_pk(
                        &cache_key,
                        &Arc::new(agg_params.clone()),
                        &agg_circuit,
                        &mut aggregation_proof.aux,
                    )
                    .await
                    .map_err(|e| e.to_string())?
            };
            let agg_instance = agg_circuit.instance();
            aggregation_proof.instance = collect_instance_hex(&agg_instance);
            let proof = {
                let time_started = Instant::now();
                #[cfg(feature = "evm-verifier")]
                let (num_instances, instances, accumulator_indices) = {
                    (
                        agg_circuit.num_instance().clone(),
                        agg_circuit.instance().clone(),
                        Some(agg_circuit.accumulator_indices()),
                    )
                };

                let v = gen_evm_proof_gwc(&agg_params, &agg_pk, agg_circuit, agg_instance);
                #[cfg(feature = "evm-verifier")]
                {
                    let deployment_code = evm_verifier_helper::gen_verifier(
                        &agg_params,
                        &agg_pk.get_vk(),
                        evm_verifier_helper::Config::kzg()
                            .with_num_instance(num_instances.clone())
                            .with_accumulator_indices(accumulator_indices),
                        num_instances,
                        evm_verifier_helper::AccumulationSchemeType::GwcType,
                    );
                    let evm_verifier_bytecode =
                        evm_verifier_helper::evm::compile_solidity(&deployment_code);
                    evm_verifier_helper::evm_verify(evm_verifier_bytecode, instances, v.clone());
                }

                aggregation_proof.aux.proof =
                    Instant::now().duration_since(time_started).as_millis() as u32;
                v
            };

            if std::env::var("PROVERD_DUMP").is_ok() {
                File::create(format!(
                    "proof-{}-agg--{:?}",
                    task_options.circuit, &circuit_config
                ))
                .unwrap()
                .write_all(&proof)
                .unwrap();
            }
            aggregation_proof.proof = proof.into();
        } else {
            let proof = gen_proof::<
                _,
                _,
                EvmTranscript<G1Affine, _, _, _>,
                EvmTranscript<G1Affine, _, _, _>,
                _,
            >(
                &circuit_param,
                &pk,
                circuit,
                circuit_instance.clone(),
                fixed_rng(),
                true,
                task_options.verify_proof,
                &mut circuit_proof.aux,
            );
            circuit_proof.proof = proof.into();
        }
    }

    Ok((circuit_config, circuit_proof, aggregation_proof))
}

macro_rules! compute_proof_wrapper {
    ($shared_state:expr, $task_options:expr, $witness:expr, $CIRCUIT:ident) => {{
        let timing = Instant::now();
        let circuit = $CIRCUIT::<
            { CIRCUIT_CONFIG.max_txs },
            { CIRCUIT_CONFIG.max_calldata },
            { CIRCUIT_CONFIG.max_rws },
            { CIRCUIT_CONFIG.max_copy_rows },
            _,
        >(&$witness, fixed_rng())?;
        let timing = Instant::now().duration_since(timing).as_millis() as u32;
        let (circuit_config, mut circuit_proof, aggregation_proof) =
            compute_proof(&$shared_state, &$task_options, CIRCUIT_CONFIG, circuit).await?;
        circuit_proof.aux.circuit = timing;
        (circuit_config, circuit_proof, aggregation_proof)
    }};
}

#[derive(Clone)]
pub struct RoState {
    // a unique identifier
    pub node_id: String,
    // a `HOSTNAME:PORT` conformant string that will be used for DNS service discovery of other
    // nodes
    pub node_lookup: Option<String>,
}

pub struct RwState {
    pub tasks: Vec<ProofRequest>,
    pub pk_cache: HashMap<String, Arc<ProverKey>>,
    /// The current active task this instance wants to obtain or is working on.
    pub pending: Option<ProofRequestOptions>,
    /// `true` if this instance started working on `pending`
    pub obtained: bool,
}

#[derive(Clone)]
pub struct SharedState {
    pub ro: RoState,
    pub rw: Arc<Mutex<RwState>>,
}

impl SharedState {
    pub fn new(node_id: String, node_lookup: Option<String>) -> SharedState {
        Self {
            ro: RoState {
                node_id,
                node_lookup,
            },
            rw: Arc::new(Mutex::new(RwState {
                tasks: Vec::new(),
                pk_cache: HashMap::new(),
                pending: None,
                obtained: false,
            })),
        }
    }

    /// Will return the result or error of the task if it's completed.
    /// Otherwise enqueues the task and returns `None`.
    /// `retry_if_error` enqueues the task again if it returned with an error
    /// before.
    pub async fn get_or_enqueue(
        &self,
        options: &ProofRequestOptions,
    ) -> Option<Result<Proofs, String>> {
        let mut rw = self.rw.lock().await;

        // task already pending or completed?
        let task = rw.tasks.iter_mut().find(|e| e.options == *options);

        if task.is_some() {
            let mut task = task.unwrap();

            if task.result.is_some() {
                if options.retry && task.result.as_ref().unwrap().is_err() {
                    log::debug!("retrying: {:#?}", task);
                    // will be a candidate in `duty_cycle` again
                    task.result = None;
                    task.edition += 1;
                } else {
                    log::debug!("completed: {:#?}", task);
                    return task.result.clone();
                }
            } else {
                log::debug!("pending: {:#?}", task);
                return None;
            }
        } else {
            // enqueue the task
            let task = ProofRequest {
                options: options.clone(),
                result: None,
                edition: 0,
            };
            log::debug!("enqueue: {:#?}", task);
            rw.tasks.push(task);
        }

        None
    }

    /// Checks if there is anything to do like:
    /// - records if a task completed
    /// - starting a new task
    /// Blocks until completion but releases the lock of `self.rw` in between.
    pub async fn duty_cycle(&self) {
        // fix the 'world' view
        if let Err(err) = self.merge_tasks_from_peers().await {
            log::error!("merge_tasks_from_peers failed with: {}", err);
            return;
        }

        let rw = self.rw.lock().await;
        if rw.pending.is_some() || rw.obtained {
            // already computing
            return;
        }
        // find a pending task
        let tasks: Vec<ProofRequestOptions> = rw
            .tasks
            .iter()
            .filter(|&e| e.result.is_none())
            .map(|e| e.options.clone())
            .collect();
        drop(rw);

        for task in tasks {
            // signals that this node wants to process this task
            log::debug!("trying to obtain {:#?}", task);
            self.rw.lock().await.pending = Some(task);

            // notify other peers
            // wrap the object because it's important to clear `pending` on error
            {
                let self_copy = self.clone();
                let obtain_task =
                    tokio::spawn(
                        async move { self_copy.obtain_task().await.expect("obtain_task") },
                    )
                    .await;

                if obtain_task.is_err() || !obtain_task.unwrap() {
                    self.rw.lock().await.pending = None;
                    log::debug!("failed to obtain task");
                    continue;
                }

                // won the race
                self.rw.lock().await.obtained = true;
                break;
            }
        }

        // needs to be cloned because of long running tasks and
        // the possibility that the task gets removed in the meantime
        let task_options = self.rw.lock().await.pending.clone();
        if task_options.is_none() {
            // nothing to do
            return;
        }

        // succesfully obtained the task
        let task_options = task_options.unwrap();
        log::info!("compute_proof: {:#?}", task_options);

        // Note: this catches any panics for the task itself but will not help in the
        // situation when the process get itself OOM killed, stack overflows etc.
        // This could be avoided by spawning a subprocess for the proof computation
        // instead.

        // spawn a task to catch panics
        let task_result: Result<Result<Proofs, String>, tokio::task::JoinError> = {
            let task_options_copy = task_options.clone();
            let self_copy = self.clone();

            tokio::spawn(async move {
                let witness = CircuitWitness::from_request(&task_options_copy)
                    .await
                    .map_err(|e| e.to_string())?;

                let (config, circuit_proof, aggregation_proof) = crate::match_circuit_params!(
                    witness.gas_used(),
                    {
                        match task_options_copy.circuit.as_str() {
                            // "pi" => {
                            //     compute_proof_wrapper!(
                            //         self_copy,
                            //         task_options_copy,
                            //         &witness,
                            //         gen_pi_circuit
                            //     )
                            // }
                            "super" => {
                                compute_proof_wrapper!(
                                    self_copy,
                                    task_options_copy,
                                    &witness,
                                    gen_super_circuit
                                )
                            }
                            // "evm" => {
                            //     compute_proof_wrapper!(
                            //         self_copy,
                            //         task_options_copy,
                            //         &witness,
                            //         gen_evm_circuit
                            //     )
                            // }
                            // "state" => compute_proof_wrapper!(
                            //     self_copy,
                            //     task_options_copy,
                            //     &witness,
                            //     gen_state_circuit
                            // ),
                            // "tx" => {
                            //     compute_proof_wrapper!(
                            //         self_copy,
                            //         task_options_copy,
                            //         &witness,
                            //         gen_tx_circuit
                            //     )
                            // }
                            // "bytecode" => compute_proof_wrapper!(
                            //     self_copy,
                            //     task_options_copy,
                            //     &witness,
                            //     gen_bytecode_circuit
                            // ),
                            // "copy" => {
                            //     compute_proof_wrapper!(
                            //         self_copy,
                            //         task_options_copy,
                            //         &witness,
                            //         gen_copy_circuit
                            //     )
                            // }
                            // "exp" => {
                            //     compute_proof_wrapper!(
                            //         self_copy,
                            //         task_options_copy,
                            //         &witness,
                            //         gen_exp_circuit
                            //     )
                            // }
                            // "keccak" => compute_proof_wrapper!(
                            //     self_copy,
                            //     task_options_copy,
                            //     &witness,
                            //     gen_keccak_circuit
                            // ),
                            _ => panic!("unknown circuit"),
                        }
                    },
                    {
                        return Err(format!(
                            "No circuit parameters found for block with gas used={}",
                            witness.gas_used()
                        ));
                    }
                );

                let res = Proofs {
                    config,
                    circuit: circuit_proof,
                    aggregation: aggregation_proof,
                    gas: witness.gas_used(),
                };

                Ok(res)
            })
            .await
        };

        // convert the JoinError to string - if applicable
        let task_result: Result<Proofs, String> = match task_result {
            Err(err) => match err.is_panic() {
                true => {
                    let panic = err.into_panic();

                    if let Some(msg) = panic.downcast_ref::<&str>() {
                        Err(msg.to_string())
                    } else if let Some(msg) = panic.downcast_ref::<String>() {
                        Err(msg.to_string())
                    } else {
                        Err("unknown panic".to_string())
                    }
                }
                false => Err(err.to_string()),
            },
            Ok(val) => val,
        };

        {
            // done, update the queue
            log::info!("task_result: {:#?}", task_result);

            let mut rw = self.rw.lock().await;
            // clear fields
            rw.pending = None;
            rw.obtained = false;
            // insert task result
            let task = rw.tasks.iter_mut().find(|e| e.options == task_options);
            if let Some(task) = task {
                // found our task, update result
                task.result = Some(task_result);
                task.edition += 1;
            } else {
                // task was already removed in the meantime,
                // assume it's obsolete and forget about it
                log::info!(
                    "task was already removed, ignoring result {:#?}",
                    task_options
                );
            }
        }
    }

    /// Returns `node_id` and `tasks` for this instance.
    /// Normally used for the rpc api.
    pub async fn get_node_information(&self) -> NodeInformation {
        NodeInformation {
            id: self.ro.node_id.clone(),
            tasks: self.rw.lock().await.tasks.clone(),
        }
    }

    /// Pulls `NodeInformation` from all other peers and
    /// merges missing or updated tasks from these peers to
    /// preserve information in case individual nodes are going to be
    /// terminated.
    ///
    /// Always returns `true` otherwise returns with error.
    pub async fn merge_tasks_from_peers(&self) -> Result<bool, String> {
        const LOG_TAG: &str = "merge_tasks_from_peers:";

        if self.ro.node_lookup.is_none() {
            return Ok(true);
        }

        let hyper_client = hyper::Client::new();
        let addrs_iter = self
            .ro
            .node_lookup
            .as_ref()
            .unwrap()
            .to_socket_addrs()
            .map_err(|e| e.to_string())?;

        for addr in addrs_iter {
            let uri = Uri::try_from(format!("http://{addr}")).map_err(|e| e.to_string())?;
            let peer: NodeInformation =
                jsonrpc_request_client(5000, &hyper_client, &uri, "info", serde_json::json!([]))
                    .await?;

            if peer.id == self.ro.node_id {
                log::debug!("{} skipping self({})", LOG_TAG, peer.id);
                continue;
            }

            log::debug!("{} merging with peer({})", LOG_TAG, peer.id);
            self.merge_tasks(&peer).await;
        }

        Ok(true)
    }

    // TODO: can this be pre-generated to a file?
    // related
    // https://github.com/zcash/halo2/issues/443
    // https://github.com/zcash/halo2/issues/449
    /// Compute or retrieve a proving key from cache.
    async fn gen_pk<C: Circuit<Fr>>(
        &self,
        cache_key: &str,
        param: &Arc<ProverParams>,
        circuit: &C,
        aux: &mut ProofResultInstrumentation,
    ) -> Result<Arc<ProverKey>, Box<dyn std::error::Error>> {
        let mut rw = self.rw.lock().await;
        if !rw.pk_cache.contains_key(cache_key) {
            // drop, potentially long running
            drop(rw);

            let vk = {
                let time_started = Instant::now();
                let vk = keygen_vk(param.as_ref(), circuit)?;
                aux.vk = Instant::now().duration_since(time_started).as_millis() as u32;
                vk
            };
            let pk = {
                let time_started = Instant::now();
                let pk = keygen_pk(param.as_ref(), vk, circuit)?;
                aux.pk = Instant::now().duration_since(time_started).as_millis() as u32;
                pk
            };
            if std::env::var("PROVERD_DUMP").is_ok() {
                pk.write(
                    &mut File::create(cache_key).unwrap(),
                    SerdeFormat::RawBytesUnchecked,
                )
                .unwrap();
            }

            let pk = Arc::new(pk);

            // acquire lock and update
            rw = self.rw.lock().await;
            rw.pk_cache.insert(cache_key.to_string(), pk);

            log::info!("ProvingKey: generated and cached key={}", cache_key);
        }

        Ok(rw.pk_cache.get(cache_key).unwrap().clone())
    }

    async fn merge_tasks(&self, node_info: &NodeInformation) {
        const LOG_TAG: &str = "merge_tasks:";
        let mut rw = self.rw.lock().await;

        for peer_task in &node_info.tasks {
            let maybe_task = rw.tasks.iter_mut().find(|e| e.options == peer_task.options);

            if let Some(existent_task) = maybe_task {
                if existent_task.edition >= peer_task.edition {
                    // fast case
                    log::debug!("{} up to date {:#?}", LOG_TAG, existent_task);
                    continue;
                }

                // update result, edition
                existent_task.edition = peer_task.edition;
                existent_task.result = peer_task.result.clone();
                log::debug!("{} updated {:#?}", LOG_TAG, existent_task);
            } else {
                // copy task
                rw.tasks.push(peer_task.clone());
                log::debug!("{} new task {:#?}", LOG_TAG, peer_task);
            }
        }
    }

    /// Tries to obtain `self.rw.pending` by querying all other peers
    /// about their current task item that resolves to either
    /// winning or losing the task depending on the algorithm.
    ///
    /// Expects `self.rw.pending` to be not `None`
    async fn obtain_task(&self) -> Result<bool, String> {
        const LOG_TAG: &str = "obtain_task:";

        let task_options = self
            .rw
            .lock()
            .await
            .pending
            .as_ref()
            .expect("pending task")
            .clone();

        if self.ro.node_lookup.is_none() {
            return Ok(true);
        }

        // resolve all other nodes for this service
        let hyper_client = hyper::Client::new();
        let addrs_iter = self
            .ro
            .node_lookup
            .as_ref()
            .unwrap()
            .to_socket_addrs()
            .map_err(|e| e.to_string())?;
        for addr in addrs_iter {
            let uri = Uri::try_from(format!("http://{addr}")).map_err(|e| e.to_string())?;
            let peer: NodeStatus =
                jsonrpc_request_client(5000, &hyper_client, &uri, "status", serde_json::json!([]))
                    .await?;

            if peer.id == self.ro.node_id {
                log::debug!("{} skipping self({})", LOG_TAG, peer.id);
                continue;
            }

            if let Some(peer_task) = peer.task {
                if peer_task == task_options {
                    // a slight chance to 'win' the task
                    if !peer.obtained && peer.id > self.ro.node_id {
                        log::debug!("{} won task against {}", LOG_TAG, peer.id);
                        // continue the race against the remaining peers
                        continue;
                    }

                    log::debug!("{} lost task against {}", LOG_TAG, peer.id);
                    // early return
                    return Ok(false);
                }
            }
        }

        // default
        Ok(true)
    }

    pub fn random_worker_id() -> String {
        // derive a (sufficiently large) random worker id
        const N: usize = 16;
        let mut arr = [0u8; N];
        thread_rng().fill(&mut arr[..]);
        let mut node_id = String::with_capacity(N * 2);
        for byte in arr {
            write!(node_id, "{byte:02x}").unwrap();
        }

        node_id
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use env_logger::Env;
    use eth_types::Address;
    use eth_types::ToBigEndian;
    use eth_types::ToWord;
    use eth_types::H256;
    use ethers_core::abi::encode;
    use ethers_core::abi::Token;
    use ethers_core::utils::keccak256;
    use hex::ToHex;

    fn parse_hash(input: &str) -> H256 {
        H256::from_slice(&hex::decode(input).expect("parse_hash"))
    }

    fn parse_address(input: &str) -> Address {
        Address::from_slice(&hex::decode(input).expect("parse_address"))
    }

    #[test]
    fn test_abi_enc_hash() {
        let meta_hash = "e7c4698134a4c5dce0c885ea9e202be298537756bb363750256ed0c5a603ff11";
        let block_hash = "b58dfe193fb44bd3b99398910ffc3da6176665617aff46bcf9bc218fb87a0ebd";
        let parent_hash = "2d6ff9593ec597e5d90752ea68f43ba69df5b89ab17eadbbdcdd3e11b7e17ea3";
        let signal_root = "25f5352342833794e6c468e5818cd88163fff61963891a7237a48567cb88b597";
        let graffiti = "6162630000000000000000000000000000000000000000000000000000000000";
        let prover = "70997970C51812dc3A010C7d01b50e0d17dc79C8";

        let pi = Token::FixedArray(vec![
            Token::FixedBytes(parse_hash(meta_hash).to_word().to_be_bytes().into()),
            Token::FixedBytes(parse_hash(parent_hash).to_word().to_be_bytes().into()),
            Token::FixedBytes(parse_hash(block_hash).to_word().to_be_bytes().into()),
            Token::FixedBytes(parse_hash(signal_root).to_word().to_be_bytes().into()),
            Token::FixedBytes(parse_hash(graffiti).to_word().to_be_bytes().into()),
            Token::Address(parse_address(prover)),
        ]);

        let buf = encode(&[pi]);
        let hash = keccak256(&buf);
        println!("hash={:?}", hash.encode_hex::<String>());
    }

    #[tokio::test]
    async fn test_dummy_proof_gen() -> Result<(), String> {
        let ss = SharedState::new("1234".to_owned(), None);
        const CIRCUIT_CONFIG: CircuitConfig = crate::match_circuit_params!(1000, CIRCUIT_CONFIG, {
            panic!();
        });
        let protocol_instance = RequestExtraInstance {
            l1_signal_service: "7a2088a1bFc9d81c55368AE168C2C02570cB814F".to_string(),
            l2_signal_service: "1000777700000000000000000000000000000007".to_string(),
            l2_contract: "1000777700000000000000000000000000000001".to_string(),
            request_meta_data: RequestMetaData {
                id: 10,
                timestamp: 1702037218,
                l1_height: 57,
                l1_hash: "73d982228d47736b4ac3079ab3e2469662ac873c6af08a46783932ca08c7d6ad"
                    .to_string(),
                l1_mix_hash: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                deposits_hash:
                    "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
                        .to_string(),
                blob_hash:
                    "569e75fc77c1a856f6daaf9e69d8a9566ca34aa47f9133711ce065a571af0cfd"
                        .to_string(),
                tx_list_byte_offset: 0,
                tx_list_byte_size: 0,
                gas_limit: 820000000,
                coinbase: "0000000000000000000000000000000000000000".to_string(),
                treasury: "df09A0afD09a63fb04ab3573922437e1e637dE8b".to_string(),
                parent_metahash: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
                ..Default::default()
            },
            block_hash: "fb9f43d074f3e889f7870aed5bf57a07d287a0444196e432153ac0c8bb526128"
                .to_string(),
            parent_hash: "35edce94199aa6d431a5229092123b222f3de42cfc1dbedeec8633efb3b8dfc5"
                .to_string(),
            signal_root: "4863d4338e270b3bd07ed68e084177b2faf9a07546dc644ed2322cbd2431f2ef"
                .to_string(),
            graffiti: "6162630000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            prover: "70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string(),
            gas_used: 412402,
            parent_gas_used: 412225,
            block_max_gas_limit: 6000000,
            max_transactions_per_block: 79,
            max_bytes_per_tx_list: 120000,
            anchor_gas_limit: 250000,
        };

        let dummy_req = ProofRequestOptions {
            circuit: "super".to_string(),
            block: protocol_instance.request_meta_data.id,
            rpc: "https://rpc.internal.taiko.xyz/".to_string(),
            protocol_instance,
            param: Some("./params".to_string()),
            aggregate: false,
            retry: true,
            mock: true,
            mock_feedback: false,
            verify_proof: true,
        };

        let witness = CircuitWitness::dummy_with_request(&dummy_req)
            .await
            .unwrap();

        let super_circuit = gen_super_circuit::<
            { CIRCUIT_CONFIG.max_txs },
            { CIRCUIT_CONFIG.max_calldata },
            { CIRCUIT_CONFIG.max_rws },
            { CIRCUIT_CONFIG.max_copy_rows },
            _,
        >(&witness, fixed_rng())
        .unwrap();

        println!("ready to compute proof");
        let proof = compute_proof(&ss, &dummy_req, CIRCUIT_CONFIG, super_circuit)
            .await
            .unwrap();
        println!("proof={:?}", proof);
        Ok(())
    }

    #[warn(dead_code)]
    fn mock_requests() -> Vec<RequestExtraInstance> {
        vec![
            RequestExtraInstance {
                l1_signal_service: "7a2088a1bFc9d81c55368AE168C2C02570cB814F".to_string(),
                l2_signal_service: "1000777700000000000000000000000000000007".to_string(),
                l2_contract: "1000777700000000000000000000000000000001".to_string(),
                request_meta_data: RequestMetaData {
                    id: 11,
                    timestamp: 1702037242,
                    l1_height: 59,
                    l1_hash: "21d59ae0428c8c52eaa9de61fbfa2e3cac88899419b126eea349d5866fb660d7"
                        .to_string(),
                    l1_mix_hash: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    deposits_hash:
                        "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
                            .to_string(),
                    blob_hash:
                        "569e75fc77c1a856f6daaf9e69d8a9566ca34aa47f9133711ce065a571af0cfd"
                            .to_string(),
                    tx_list_byte_offset: 0,
                    tx_list_byte_size: 0,
                    gas_limit: 820000000,
                    coinbase: "0000000000000000000000000000000000000000".to_string(),
                    treasury: "df09A0afD09a63fb04ab3573922437e1e637dE8b".to_string(),
                    parent_metahash: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    ..Default::default()
                },
                block_hash: "3a17d93967db151806ea94dd6ea73f4e7ba114953589fc1dfbcc51d5f803ec14"
                    .to_string(),
                parent_hash: "fb9f43d074f3e889f7870aed5bf57a07d287a0444196e432153ac0c8bb526128"
                    .to_string(),
                signal_root: "4863d4338e270b3bd07ed68e084177b2faf9a07546dc644ed2322cbd2431f2ef"
                    .to_string(),
                graffiti: "6162630000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                prover: "70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string(),
                gas_used: 412579,
                parent_gas_used: 412402,
                block_max_gas_limit: 6000000,
                max_transactions_per_block: 79,
                max_bytes_per_tx_list: 120000,
                anchor_gas_limit: 250000,
            },
            RequestExtraInstance {
                l1_signal_service: "7a2088a1bFc9d81c55368AE168C2C02570cB814F".to_string(),
                l2_signal_service: "1000777700000000000000000000000000000007".to_string(),
                l2_contract: "1000777700000000000000000000000000000001".to_string(),
                request_meta_data: RequestMetaData {
                    id: 1027,
                    timestamp: 1702060702,
                    l1_height: 2014,
                    l1_hash: "8681386ff9895d0c840337041871fe014d7d8406b6ec922a5c362d9ef9b31a81"
                        .to_string(),
                    l1_mix_hash: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    deposits_hash:
                        "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
                            .to_string(),
                    blob_hash:
                        "569e75fc77c1a856f6daaf9e69d8a9566ca34aa47f9133711ce065a571af0cfd"
                            .to_string(),
                    tx_list_byte_offset: 0,
                    tx_list_byte_size: 0,
                    gas_limit: 820000000,
                    coinbase: "0000000000000000000000000000000000000000".to_string(),
                    treasury: "df09A0afD09a63fb04ab3573922437e1e637dE8b".to_string(),
                    parent_metahash: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                    ..Default::default()
                },
                block_hash: "e2d57a162e9a0ffed195a20ee1eb5a23b6e5c17207c2e781d9222a774a1fefaf"
                    .to_string(),
                parent_hash: "2c40f4c8e4c339ac8b24ef1cd3127bbf89ec3497a7d2ecd12d2095d32d14ae90"
                    .to_string(),
                signal_root: "aa4cbc3520c2fe466bcd301e194f64430793c731c97d4819c55306afbb435627"
                    .to_string(),
                graffiti: "6162630000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                prover: "70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string(),
                gas_used: 606310,
                parent_gas_used: 606310,
                block_max_gas_limit: 6000000,
                max_transactions_per_block: 79,
                max_bytes_per_tx_list: 120000,
                anchor_gas_limit: 250000,
            },
        ]
    }

    #[tokio::test]
    async fn test_with_high_degree() -> Result<(), String> {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

        let ss = SharedState::new("1234".to_owned(), None);
        const CIRCUIT_CONFIG: CircuitConfig =
            crate::match_circuit_params!(10001, CIRCUIT_CONFIG, {
                panic!();
            });
        let protocol_instance = mock_requests()[1].clone();
        let dummy_req = ProofRequestOptions {
            circuit: "super".to_string(),
            block: protocol_instance.request_meta_data.id,
            rpc: "https://rpc.internal.taiko.xyz/".to_string(),
            protocol_instance,
            param: Some("./params".to_string()),
            aggregate: true,
            retry: true,
            mock: false,
            mock_feedback: false,
            verify_proof: true,
        };

        let witness = CircuitWitness::from_request(&dummy_req).await.unwrap();

        let super_circuit = gen_super_circuit::<
            { CIRCUIT_CONFIG.max_txs },
            { CIRCUIT_CONFIG.max_calldata },
            { CIRCUIT_CONFIG.max_rws },
            { CIRCUIT_CONFIG.max_copy_rows },
            _,
        >(&witness, fixed_rng())
        .unwrap();

        println!("ready to compute proof");
        let proof = compute_proof(&ss, &dummy_req, CIRCUIT_CONFIG, super_circuit)
            .await
            .unwrap();
        println!("proof={:?}", proof);
        Ok(())
    }
}
