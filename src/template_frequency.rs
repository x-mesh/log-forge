use rand::{Rng, distributions::Distribution, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationRule {
    pub condition_field: String,
    pub condition_value: String,
    #[serde(default)]
    pub condition_operator: ConditionOperator,
    pub target_field: String,
    pub target_distribution: BTreeMap<String, f64>,
    #[serde(default = "one")]
    pub probability: f64,
}
#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    Equal,
    GreaterThan,
}

impl CorrelationRule {
    pub(crate) fn validate_condition(&self) -> Result<(), String> {
        if matches!(self.condition_operator, ConditionOperator::GreaterThan) {
            self.condition_value
                .parse::<i64>()
                .map_err(|_| "invalid integer correlation threshold".to_owned())?;
        }
        Ok(())
    }

    fn matches(&self, context: &HashMap<String, String>) -> Result<bool, String> {
        let Some(value) = context.get(&self.condition_field) else {
            return Ok(false);
        };
        match self.condition_operator {
            ConditionOperator::Equal => Ok(value == &self.condition_value),
            ConditionOperator::GreaterThan => {
                let value = value.parse::<i64>().map_err(|_| {
                    format!(
                        "invalid integer in correlation field {}",
                        self.condition_field
                    )
                })?;
                let threshold = self
                    .condition_value
                    .parse::<i64>()
                    .map_err(|_| "invalid integer correlation threshold".to_owned())?;
                Ok(value > threshold)
            }
        }
    }
}
fn one() -> f64 {
    1.0
}

pub(crate) type Frequencies = BTreeMap<String, BTreeMap<String, f64>>;

pub(crate) fn validate_distribution(distribution: &BTreeMap<String, f64>) -> Result<(), String> {
    if distribution
        .keys()
        .any(|value| value.is_empty() || value.contains(['\n', '\r']))
    {
        return Err("invalid distribution value".into());
    }
    crate::weights::checked(distribution.values().copied()).map_err(|error| error.to_string())?;
    Ok(())
}
pub(crate) fn validate(
    template: &str,
    frequencies: &Frequencies,
    rules: &[CorrelationRule],
) -> Result<(), String> {
    let mut positions = HashMap::new();
    for part in crate::template_syntax::parse(template) {
        let spec = match part? {
            crate::template_syntax::Part::Text(_) => continue,
            crate::template_syntax::Part::Field(spec) => spec,
        };
        let name = spec.split(':').next().unwrap_or("");
        let index = positions.len();
        if let Some((previous, _)) = positions.get(name) {
            if *previous != spec
                && (frequencies.contains_key(name)
                    || rules
                        .iter()
                        .any(|rule| rule.condition_field == name || rule.target_field == name))
            {
                return Err(format!("controlled field {name} uses conflicting options"));
            }
        } else {
            positions.insert(name, (spec, index));
        }
    }
    for (name, distribution) in frequencies {
        if !positions.contains_key(name.as_str()) {
            return Err(format!("frequency field not in template: {name}"));
        }
        validate_distribution(distribution)?;
    }
    for rule in rules {
        rule.validate_condition()?;
        if !rule.probability.is_finite() || !(0.0..=1.0).contains(&rule.probability) {
            return Err("correlation probability must be between 0 and 1".into());
        }
        let (_, condition) = positions
            .get(rule.condition_field.as_str())
            .ok_or("correlation condition field not in template")?;
        let (_, target) = positions
            .get(rule.target_field.as_str())
            .ok_or("correlation target field not in template")?;
        if condition >= target {
            return Err("correlation condition must appear before target".into());
        }
        validate_distribution(&rule.target_distribution)?;
    }
    Ok(())
}
fn choose(distribution: &BTreeMap<String, f64>, rng: &mut StdRng) -> Result<String, String> {
    let weights = crate::weights::checked(distribution.values().copied())
        .map_err(|error| error.to_string())?;
    distribution
        .keys()
        .nth(weights.sample(rng))
        .cloned()
        .ok_or_else(|| "empty distribution".into())
}
pub(crate) fn value(
    name: &str,
    context: &HashMap<String, String>,
    frequencies: &Frequencies,
    rules: &[CorrelationRule],
    rng: &mut StdRng,
) -> Result<Option<String>, String> {
    for rule in rules {
        if rule.target_field == name && rule.matches(context)? && rng.gen_bool(rule.probability) {
            return choose(&rule.target_distribution, rng).map(Some);
        }
    }
    frequencies
        .get(name)
        .map(|distribution| choose(distribution, rng))
        .transpose()
}
