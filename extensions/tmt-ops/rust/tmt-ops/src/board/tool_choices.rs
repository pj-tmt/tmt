//! Tool and model suggestions from public observations; no provider registry.
use super::inventory::Inventory;
use std::collections::{BTreeMap, BTreeSet};
pub(in crate::board) fn observed_model(agent: &super::inventory::Agent) -> Option<&str> {
    agent.parameters["resume"]["observedModel"]["model"]
        .as_str()
        .filter(|model| !model.is_empty())
}

pub(in crate::board) fn observed(
    inventory: &Inventory,
) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    let mut models = BTreeMap::<String, Vec<String>>::new();
    for agent in inventory.agents.values() {
        if let (Some(tool), Some(model)) = (&agent.runtime, observed_model(agent)) {
            let options = models.entry(tool.clone()).or_default();
            if !options.iter().any(|option| option == model) {
                options.push(model.to_owned());
            }
        }
    }
    for options in models.values_mut() {
        options.sort();
    }
    let tools = inventory
        .agents
        .values()
        .filter_map(|a| a.runtime.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    (tools, models)
}
