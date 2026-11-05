use rand::{Rng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TraceConfig {
    pub enabled: bool,
    pub trace_id_header: String,
    pub span_id_header: String,
    pub service_dependencies: BTreeMap<String, Vec<String>>,
}
impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trace_id_header: "X-Trace-Id".into(),
            span_id_header: "X-Span-Id".into(),
            service_dependencies: BTreeMap::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub trace_header: String,
    pub span_header: String,
}

pub(crate) struct TracePlanner {
    graph: BTreeMap<usize, Vec<usize>>,
    services: BTreeSet<usize>,
    pending: VecDeque<(usize, TraceContext)>,
    trace_header: String,
    span_header: String,
}

fn check_cycles(
    node: usize,
    graph: &BTreeMap<usize, Vec<usize>>,
    visiting: &mut BTreeSet<usize>,
    visited: &mut BTreeSet<usize>,
) -> io::Result<()> {
    if visiting.contains(&node) {
        return Err(io::Error::other("service dependency cycle"));
    }
    if visited.contains(&node) {
        return Ok(());
    }
    visiting.insert(node);
    if let Some(children) = graph.get(&node) {
        for child in children {
            check_cycles(*child, graph, visiting, visited)?;
        }
    }
    visiting.remove(&node);
    visited.insert(node);
    Ok(())
}

impl TracePlanner {
    pub fn new(config: &TraceConfig, names: &[(usize, String)]) -> io::Result<Self> {
        if names.len() > 1024 {
            return Err(io::Error::other("tracing supports at most 1024 services"));
        }
        for header in [&config.trace_id_header, &config.span_id_header] {
            if header.is_empty()
                || !header
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || ["trace_id", "span_id", "parent_span_id"].contains(&header.as_str())
            {
                return Err(io::Error::other("invalid trace header name"));
            }
        }
        if config
            .trace_id_header
            .eq_ignore_ascii_case(&config.span_id_header)
        {
            return Err(io::Error::other("trace and span headers must differ"));
        }
        let mut indices = BTreeMap::new();
        for (index, name) in names {
            if indices.insert(name.clone(), *index).is_some() {
                return Err(io::Error::other(format!("duplicate service name: {name}")));
            }
        }
        let services = indices.values().copied().collect();
        let mut graph = BTreeMap::new();
        for (name, children) in &config.service_dependencies {
            let index = *indices
                .get(name)
                .ok_or_else(|| io::Error::other(format!("unknown tracing service: {name}")))?;
            let mut selected = Vec::new();
            for child in children {
                selected.push(*indices.get(child).ok_or_else(|| {
                    io::Error::other(format!("unknown tracing service: {child}"))
                })?);
            }
            graph.insert(index, selected);
        }
        let mut visited = BTreeSet::new();
        for index in indices.values() {
            check_cycles(*index, &graph, &mut BTreeSet::new(), &mut visited)?;
        }
        Ok(Self {
            graph,
            services,
            pending: VecDeque::new(),
            trace_header: config.trace_id_header.clone(),
            span_header: config.span_id_header.clone(),
        })
    }
    pub fn next(
        &mut self,
        selected: Option<usize>,
        rng: &mut StdRng,
    ) -> io::Result<Option<(usize, Option<TraceContext>)>> {
        if let Some((index, context)) = self.pending.pop_front() {
            return Ok(Some((index, Some(context))));
        }
        let Some(index) = selected else {
            return Ok(None);
        };
        if !self.services.contains(&index) {
            return Ok(Some((index, None)));
        }
        let trace_id = format!("{:032x}", rng.r#gen::<u128>() | 1);
        let mut stack = vec![(index, None::<String>)];
        while let Some((current, parent)) = stack.pop() {
            if self.pending.len() >= 1024 {
                self.pending.clear();
                return Err(io::Error::other("trace exceeds 1024 spans"));
            }
            let span_id = format!("{:016x}", rng.r#gen::<u64>() | 1);
            let context = TraceContext {
                trace_id: trace_id.clone(),
                span_id: span_id.clone(),
                parent_span_id: parent,
                trace_header: self.trace_header.clone(),
                span_header: self.span_header.clone(),
            };
            self.pending.push_back((current, context));
            if let Some(children) = self.graph.get(&current) {
                for child in children.iter().rev() {
                    stack.push((*child, Some(span_id.clone())));
                }
            }
        }
        Ok(self
            .pending
            .pop_front()
            .map(|(index, context)| (index, Some(context))))
    }
}
