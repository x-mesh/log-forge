use crate::app::compat::GoloopNetwork;
use rand::{Rng, rngs::StdRng};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Deserialize)]
struct PatternData {
    log_patterns: BTreeMap<String, Vec<String>>,
    source_files: BTreeMap<String, Vec<String>>,
    sub_modules: BTreeMap<String, Vec<String>>,
}
static DATA: LazyLock<PatternData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/goloop_patterns.json"))
        .expect("valid goloop patterns")
});

pub(crate) fn load_patterns() {
    LazyLock::force(&DATA);
}

pub(crate) struct Record {
    pub source: String,
    pub message: String,
    pub submodule: String,
    pub sequence: serde_json::Value,
}
fn pick<'a>(values: &'a [String], rng: &mut StdRng) -> &'a str {
    &values[rng.gen_range(0..values.len())]
}
fn hex(rng: &mut StdRng) -> String {
    format!("{:032x}{:032x}", rng.r#gen::<u128>(), rng.r#gen::<u128>())
}
fn address(rng: &mut StdRng) -> String {
    format!("hx{:032x}{:08x}", rng.r#gen::<u128>(), rng.r#gen::<u32>())
}

pub(crate) fn generate(
    module: &str,
    height: u64,
    network: &GoloopNetwork,
    network_id: &str,
    revision: u32,
    rng: &mut StdRng,
) -> Record {
    let patterns = DATA.log_patterns.get(module);
    let pattern = patterns.map_or("node event", |values| pick(values, rng));
    let source_file = DATA
        .source_files
        .get(module)
        .map_or("main.go", |values| pick(values, rng));
    let submodule = DATA
        .sub_modules
        .get(module)
        .map_or("", |values| pick(values, rng))
        .to_owned();
    let sequence = if rng.gen_bool(0.9) {
        serde_json::json!(1)
    } else {
        serde_json::json!("-")
    };
    let (source, pattern) = if let Some((head, tail)) = pattern.split_once(' ') {
        if head.split_once(':').is_some_and(|(file, line)| {
            file.ends_with(".go") && line.chars().all(|c| c.is_ascii_digit())
        }) {
            (head.to_owned(), tail)
        } else {
            (
                format!("{source_file}:{}", rng.gen_range(50..=800)),
                pattern,
            )
        }
    } else {
        (
            format!("{source_file}:{}", rng.gen_range(50..=800)),
            pattern,
        )
    };
    let message = match module {
        "CS" if pattern == "ReceiveBlock Height:" => format!("{pattern}{height}"),
        "CS" if pattern.starts_with("OnReceive(msg:") => format!(
            "{pattern}H:{height},R:0}}, from:{:08x}..)",
            rng.r#gen::<u32>()
        ),
        "CS" => format!(
            "{pattern} round={} height={height}",
            rng.gen_range(1..=9999)
        ),
        "BM" if pattern.ends_with('(') => format!("{pattern}{}) height={height}", hex(rng)),
        "BM" if pattern.ends_with('=') => format!("{pattern}{height}"),
        "BM" => format!("{pattern} height={height}"),
        "SV" if pattern.ends_with("height=") => format!(
            "{pattern}{height} csi=ConsensusInfo(proposer={},voters=[])",
            address(rng)
        ),
        "SV" if pattern.ends_with("total=") => format!(
            "{pattern}{:.3}ms world={:.3}ms receipts={}µs",
            rng.gen_range(1.0..15.0),
            rng.gen_range(1.0..12.0),
            rng.gen_range(200..=800)
        ),
        "SV" if pattern == "Transactions:" => format!(
            "{pattern} {} Elapsed: {:.3} ms TPS: {:.2}",
            rng.gen_range(1..=5),
            rng.gen_range(0.5..12.0),
            rng.gen_range(80.0..2000.0)
        ),
        "SV" if pattern.ends_with("waiters_before=") => format!(
            "{pattern}{} waiters_after={}",
            rng.gen_range(0..=1),
            rng.gen_range(0..=1)
        ),
        "SV" if pattern.ends_with("bits=") => {
            format!("{pattern}{},peer={})", rng.gen_range(10..=15), address(rng))
        }
        "SV" if pattern.ends_with("txs=") => {
            format!("{pattern}{},to={})", rng.gen_range(1..=15), address(rng))
        }
        "NM" => {
            let peers = [
                "18.118.232.28",
                "13.37.8.70",
                "13.55.198.141",
                "65.21.202.45",
                "138.197.69.76",
                "44.237.21.74",
                "51.195.190.28",
                "95.179.159.177",
                "139.180.142.57",
                "52.196.159.184",
                "20.237.243.215",
                "66.42.63.164",
                "18.142.22.234",
                "3.215.75.154",
                "18.130.92.113",
                "52.77.115.223",
            ];
            let peer = if network.use_real_ips {
                peers[rng.gen_range(0..peers.len())].to_owned()
            } else {
                format!(
                    "{}.{}.{}.{}",
                    rng.gen_range(1..=255),
                    rng.gen_range(1..=255),
                    rng.gen_range(1..=255),
                    rng.gen_range(1..=255)
                )
            };
            if pattern.ends_with('{') {
                format!(
                    "{pattern}id:{}, conn:{}:{}->{peer}:{}, addr:{peer}:{}}} sub=p2p",
                    address(rng),
                    network.local_ip,
                    rng.gen_range(30000..=60000),
                    network.p2p_port,
                    network.p2p_port
                )
            } else {
                format!(
                    "{pattern} conn={}:{}->{peer}:{} peer={peer}:{}",
                    network.local_ip,
                    rng.gen_range(30000..=60000),
                    network.p2p_port,
                    network.p2p_port
                )
            }
        }
        "fastsync" if pattern.ends_with("RequestID:") => format!(
            "{pattern}{} Height:{height} peer:{:08x}",
            rng.gen_range(65540..=65600),
            rng.r#gen::<u32>()
        ),
        "fastsync" => format!("{pattern} {} height={height}", rng.gen_range(20..=60)),
        "TP" if pattern.contains("handleSignatureResponse") => format!(
            "{pattern} {:.6}ms DefaultRttAccuracy 10ms sub=authenticator",
            rng.gen_range(5.0..20.0)
        ),
        "IC" => format!("{pattern} network_id={network_id} revision={revision}"),
        "TX" => format!("{pattern} hash={} from={}", hex(rng), address(rng)),
        "ST" | "CH" => format!("{pattern} height={height} hash={}", hex(rng)),
        "P2P" | "NT" => format!("{pattern} peer={} port={}", address(rng), network.p2p_port),
        "RP" => format!("{pattern} method=icx_getBlock"),
        "EE" => format!(
            "{pattern} address={} gas_used={}",
            address(rng),
            rng.gen_range(21000..=500000)
        ),
        "PL" => format!("{pattern} plugin=score address={}", address(rng)),
        "DB" => format!("{pattern} path=/data/db/chain height={height}"),
        "SM" => format!(
            "{pattern} service=json-rpc port={}",
            rng.gen_range(7000..=9000)
        ),
        _ => pattern.to_owned(),
    };
    Record {
        source,
        message,
        submodule,
        sequence,
    }
}
