use std::collections::HashMap;
use levenshtein_automata::{LevenshteinAutomatonBuilder, DFA, Distance};
use crate::core::error::Result;

/// Automaton for fuzzy matching with edit distance
pub struct FuzzyAutomaton {
    /// The target term to match
    term: String,

    /// Maximum allowed edit distance (typically 1-2)
    max_edit_distance: u8,

    /// Allow character transpositions (teh → the)
    transpositions: bool,

    /// Built DFA for matching
    dfa: Option<DFA>,
}

impl FuzzyAutomaton {
    pub fn new(term: String, max_edit_distance: u8) -> Self {
        Self {
            term,
            max_edit_distance,
            transpositions: true,
            dfa: None,
        }
    }

    /// Build the DFA for fuzzy matching
    pub fn build(&mut self) -> Result<()> {
        let lev_builder = LevenshteinAutomatonBuilder::new(
            self.max_edit_distance,
            self.transpositions,
        );

        self.dfa = Some(lev_builder.build_dfa(&self.term));
        Ok(())
    }

    /// Check if a candidate matches within edit distance
    pub fn matches(&self, candidate: &str) -> bool {
        if let Some(dfa) = &self.dfa {
            let mut state = dfa.initial_state();

            for &byte in candidate.as_bytes() {
                state = dfa.transition(state, byte);
            }

            match dfa.distance(state) {
                Distance::Exact(d) if d <= self.max_edit_distance => true,
                _ => false,
            }
        } else {
            // Fallback to simple edit distance
            self.edit_distance(candidate) <= self.max_edit_distance as usize
        }
    }

    /// Calculate Levenshtein distance (fallback)
    pub fn edit_distance(&self, other: &str) -> usize {
        let a = self.term.as_bytes();
        let b = other.as_bytes();
        let len_a = a.len();
        let len_b = b.len();

        if len_a == 0 {
            return len_b;
        }
        if len_b == 0 {
            return len_a;
        }

        let mut prev_row: Vec<usize> = (0..=len_b).collect();
        let mut curr_row = vec![0; len_b + 1];

        for i in 1..=len_a {
            curr_row[0] = i;

            for j in 1..=len_b {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };

                curr_row[j] = std::cmp::min(
                    std::cmp::min(
                        prev_row[j] + 1,     // deletion
                        curr_row[j - 1] + 1, // insertion
                    ),
                    prev_row[j - 1] + cost,  // substitution
                );

                // Check for transposition
                if self.transpositions && i > 1 && j > 1
                    && a[i - 1] == b[j - 2]
                    && a[i - 2] == b[j - 1]
                {
                    curr_row[j] = std::cmp::min(
                        curr_row[j],
                        prev_row[j - 1] + cost,
                    );
                }
            }

            std::mem::swap(&mut prev_row, &mut curr_row);
        }

        prev_row[len_b]
    }
}

pub struct LevenshteinDFA {
    /// DFA states
    states: Vec<DFAState>,

    /// State transitions: (state_id, char) -> next_state_id
    transitions: HashMap<(StateId, u8), StateId>,

    /// Maximum edit distance
    max_distance: u8,
}

#[derive(Clone, Debug)]
struct DFAState {
    id: StateId,
    is_final: bool,
    distance: u8,
}

type StateId = usize;

impl LevenshteinDFA {
    pub fn build(pattern: &str, max_distance: u8) -> Self {
        let mut dfa = Self {
            states: vec![],
            transitions: HashMap::new(),
            max_distance,
        };

        // Build NFA first, then convert to DFA
        dfa.build_from_pattern(pattern);
        dfa
    }

    fn build_from_pattern(&mut self, pattern: &str) {
        // Create initial state
        self.states.push(DFAState {
            id: 0,
            is_final: false,
            distance: 0,
        });

        // Build states for each position in pattern
        for (pos, ch) in pattern.chars().enumerate() {
            for dist in 0..=self.max_distance {
                let state_id = self.get_or_create_state(pos, dist);

                // Match transition
                if dist == 0 {
                    let next_state = self.get_or_create_state(pos + 1, dist);
                    self.transitions.insert((state_id, ch as u8), next_state);
                }

                // Insertion
                if dist < self.max_distance {
                    for c in 0u8..=127 {
                        let next_state = self.get_or_create_state(pos, dist + 1);
                        self.transitions.entry((state_id, c)).or_insert(next_state);
                    }
                }

                // Deletion
                if dist < self.max_distance && pos < pattern.len() {
                    let next_state = self.get_or_create_state(pos + 1, dist + 1);
                    self.transitions.entry((state_id, 0)).or_insert(next_state);
                }

                // Substitution
                if dist < self.max_distance {
                    for c in 0u8..=127 {
                        if c != ch as u8 {
                            let next_state = self.get_or_create_state(pos + 1, dist + 1);
                            self.transitions.entry((state_id, c)).or_insert(next_state);
                        }
                    }
                }
            }
        }

        // Mark final states
        let pattern_len = pattern.len();
        for state in &mut self.states {
            if state.id >= pattern_len * (self.max_distance as usize + 1) {
                state.is_final = true;
            }
        }
    }

    fn get_or_create_state(&mut self, position: usize, distance: u8) -> StateId {
        let id = position * (self.max_distance as usize + 1) + distance as usize;

        if id >= self.states.len() {
            self.states.resize(id + 1, DFAState {
                id: 0,
                is_final: false,
                distance: 0,
            });

            self.states[id] = DFAState {
                id,
                is_final: false,
                distance,
            };
        }

        id
    }

    /// Check if text matches within edit distance
    pub fn matches(&self, text: &str) -> Option<u8> {
        let mut current_state = 0;

        for ch in text.bytes() {
            if let Some(&next_state) = self.transitions.get(&(current_state, ch)) {
                current_state = next_state;
            } else {
                // No valid transition - text doesn't match
                return None;
            }
        }

        if current_state < self.states.len() && self.states[current_state].is_final {
            Some(self.states[current_state].distance)
        } else {
            None
        }
    }
}