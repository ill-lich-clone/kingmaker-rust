use kingmaker_core::ProvinceId;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Debug, Clone, Default)]
pub struct SupplyNetwork {
    pub adj: HashMap<ProvinceId, Vec<(ProvinceId, f64)>>,
}

impl SupplyNetwork {
    pub fn add_route(&mut self, from: ProvinceId, to: ProvinceId, distance: f64) {
        self.adj.entry(from).or_default().push((to, distance));
        self.adj.entry(to).or_default().push((from, distance));
    }
}

#[derive(Debug, Clone)]
pub struct SupplyRoute {
    pub provinces: Vec<ProvinceId>,
    pub total_cost: f64,
}

#[derive(Copy, Clone, PartialEq)]
struct NodeState {
    cost: f64,
    node: ProvinceId,
}

impl Eq for NodeState {}

impl Ord for NodeState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for NodeState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn find_supply_route(
    network: &SupplyNetwork,
    start: ProvinceId,
    goal: ProvinceId,
) -> Option<SupplyRoute> {
    let mut dist: HashMap<ProvinceId, f64> = HashMap::new();
    let mut prev: HashMap<ProvinceId, ProvinceId> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(start, 0.0);
    heap.push(NodeState {
        cost: 0.0,
        node: start,
    });

    while let Some(NodeState { cost, node }) = heap.pop() {
        if node == goal {
            let mut path = vec![goal];
            let mut curr = goal;
            while let Some(&p) = prev.get(&curr) {
                path.push(p);
                curr = p;
            }
            path.reverse();
            return Some(SupplyRoute {
                provinces: path,
                total_cost: cost,
            });
        }

        if cost > *dist.get(&node).unwrap_or(&f64::INFINITY) {
            continue;
        }

        if let Some(neighbors) = network.adj.get(&node) {
            for &(next, weight) in neighbors {
                let next_cost = cost + weight;
                if next_cost < *dist.get(&next).unwrap_or(&f64::INFINITY) {
                    dist.insert(next, next_cost);
                    prev.insert(next, node);
                    heap.push(NodeState {
                        cost: next_cost,
                        node: next,
                    });
                }
            }
        }
    }

    None
}
