use num_bigint::{BigUint, ToBigUint};
use num_traits::Zero;
use serde::{Serialize, Serializer};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize)]
pub struct StorageUpdate {
    #[serde(serialize_with = "serialize_biguint")]
    pub key: BigUint,
    #[serde(serialize_with = "serialize_biguint")]
    pub value: BigUint,
}

#[derive(Debug)]
pub struct ContractUpdate {
    pub address: BigUint,
    pub nonce: u64,
    pub new_class_hash: Option<BigUint>, // Some only if class updated
    pub storage_updates: Vec<StorageUpdate>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClassDeclaration {
    #[serde(serialize_with = "serialize_biguint")]
    pub class_hash: BigUint,
    #[serde(serialize_with = "serialize_biguint")]
    pub compiled_class_hash: BigUint,
}

#[derive(Debug)]
pub struct StateDiff {
    pub contract_updates: Vec<ContractUpdate>,
    pub class_declarations: Vec<ClassDeclaration>,
}

// adapted from majin-blob
fn serialize_biguint<S>(biguint: &BigUint, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let s = convert_biguint(biguint);
    serializer.serialize_str(&s)
}

fn convert_biguint(biguint: &BigUint) -> String {
    format!("0x{}", biguint.to_str_radix(16))
}

impl ContractUpdate {
    pub fn to_contract_storage_diff_item(&self) -> Value {
        json!({
            "address": convert_biguint(&self.address),
            "storage_entries": self.storage_updates.clone(),
        })
    }

    pub fn to_deployed_contract_item(&self) -> Value {
        let class_hash = self.new_class_hash.as_ref().map(|h| convert_biguint(h));
        json!({
            "address": convert_biguint(&self.address),
            "class_hash": class_hash,
        })
    }

    pub fn to_nonce_item(&self) -> Value {
        let n = self.nonce.to_biguint().unwrap();
        json!({
            "contract_address": convert_biguint(&self.address),
            "nonce": convert_biguint(&n),
        })
    }
}

impl StateDiff {
    // Not quite the "STATE_DIFF" RPC format, as 1) the source for
    // "deprecated_declared_classes" isn't clear and 2) the
    // distinction between "deployed_contracts" and "replaced_classes"
    // apparently isn't represented in input data...
    pub fn to_json_state_diff(&self) -> Value {
        let storage_diffs: Vec<Value> = self
            .contract_updates
            .iter()
            .filter(|cu| cu.storage_updates.len() > 0)
            .map(|cu| cu.to_contract_storage_diff_item())
            .collect();
        let deployed_contracts: Vec<Value> = self
            .contract_updates
            .iter()
            .filter(|cu| cu.new_class_hash.is_some())
            .map(|cu| cu.to_deployed_contract_item())
            .collect();
        let nonces: Vec<Value> = self
            .contract_updates
            .iter()
            .filter(|cu| !cu.nonce.is_zero())
            .map(|cu| cu.to_nonce_item())
            .collect();
        json!({
            "storage_diffs": storage_diffs,
            "declared_classes": self.class_declarations.clone(),
            "deployed_or_replaced": deployed_contracts,
            "nonces": nonces,
        })
    }
}

#[cfg(test)]
mod tests {
    use num_bigint::{BigUint, ToBigUint};
    use num_traits::Num;

    use super::{ClassDeclaration, ContractUpdate, StorageUpdate};

    #[test]
    fn storage_update() {
        let su = StorageUpdate {
            key: 1u32.to_biguint().unwrap(),
            value: 20u32.to_biguint().unwrap(),
        };
        let j = serde_json::to_string(&su).unwrap();
        assert_eq!(j.to_string(), "{\"key\":\"0x1\",\"value\":\"0x14\"}");
    }

    #[test]
    fn contract_update() {
        let cu = ContractUpdate {
            address: 42u32.to_biguint().unwrap(),
            nonce: Default::default(),
            new_class_hash: Some(37u32.to_biguint().unwrap()),
            storage_updates: vec![StorageUpdate {
                key: 1u32.to_biguint().unwrap(),
                value: 20u32.to_biguint().unwrap(),
            }],
        };
        let sdi = cu.to_contract_storage_diff_item();
        assert_eq!(
            sdi.to_string(),
            "{\"address\":\"0x2a\",\"storage_entries\":[{\"key\":\"0x1\",\"value\":\"0x14\"}]}"
        );

        let dci = cu.to_deployed_contract_item();
        assert_eq!(
            dci.to_string(),
            "{\"address\":\"0x2a\",\"class_hash\":\"0x25\"}"
        );

        let ni = cu.to_nonce_item();
        assert_eq!(
            ni.to_string(),
            "{\"contract_address\":\"0x2a\",\"nonce\":\"0x0\"}"
        );
    }

    #[test]
    fn class_declaration() {
        let ch = BigUint::from_str_radix(
            "36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f",
            16,
        )
        .unwrap();
        let cc = ClassDeclaration {
            class_hash: ch,
            compiled_class_hash: 0u32.to_biguint().unwrap(),
        };
        let j = serde_json::to_string(&cc).unwrap();
        assert_eq!(
            j.to_string(),
            "{\"class_hash\":\"0x36078334509b514626504edc9fb252328d1a240e4e948bef8d0c08dff45927f\",\"compiled_class_hash\":\"0x0\"}"
        );
    }
}
