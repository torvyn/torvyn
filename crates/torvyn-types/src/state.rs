//! State machine types for flow lifecycle and resource ownership.
//!
//! Both [`FlowState`] and [`ResourceState`] include transition validation
//! methods that enforce the legal transitions defined in the HLI documents.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// InvalidTransition
// ---------------------------------------------------------------------------

/// Error returned when an illegal state transition is attempted.
///
/// # Examples
/// ```
/// use torvyn_types::{FlowState, InvalidTransition};
///
/// let result = FlowState::Running.transition_to(FlowState::Created);
/// assert!(result.is_err());
/// let err = result.unwrap_err();
/// assert!(format!("{}", err).contains("Running"));
/// assert!(format!("{}", err).contains("Created"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidTransition {
    /// The state machine type (e.g., "FlowState", "ResourceState").
    pub machine: &'static str,
    /// The current state.
    pub from: String,
    /// The attempted target state.
    pub to: String,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} transition: '{}' \u{2192} '{}' is not permitted. \
             Check the state machine documentation for valid transitions.",
            self.machine, self.from, self.to
        )
    }
}

impl std::error::Error for InvalidTransition {}

// ---------------------------------------------------------------------------
// FlowState
// ---------------------------------------------------------------------------

/// Flow lifecycle state machine.
///
/// Per Doc 04, Section 10.1: 8 states with defined legal transitions.
/// This is the label-only version in `torvyn-types`. The reactor crate
/// provides an extended version with associated data (stats, error info).
///
/// State transition diagram:
/// ```text
///   Created -> Validated -> Instantiated -> Running -> Draining -> Completed
///   Created -> Failed                                            -> Cancelled
///   Validated -> Failed                                          -> Failed
///   Running -> Draining -> Failed
/// ```
///
/// # Examples
/// ```
/// use torvyn_types::FlowState;
///
/// let state = FlowState::Created;
/// assert!(state.can_transition_to(&FlowState::Validated));
/// assert!(!state.can_transition_to(&FlowState::Running));
///
/// let new_state = state.transition_to(FlowState::Validated).unwrap();
/// assert_eq!(new_state, FlowState::Validated);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum FlowState {
    /// Flow definition has been submitted but not yet validated.
    Created,
    /// Contracts and capabilities have been validated.
    Validated,
    /// Components have been instantiated and streams are connected.
    Instantiated,
    /// The flow is actively processing stream elements.
    Running,
    /// The flow is draining remaining elements after a completion
    /// or cancellation signal.
    Draining,
    /// The flow completed successfully.
    Completed,
    /// The flow was cancelled by operator or policy.
    Cancelled,
    /// The flow failed due to an unrecoverable error.
    Failed,
}

impl FlowState {
    /// Returns `true` if transitioning from `self` to `target` is legal.
    ///
    /// Legal transitions (per Doc 04, Section 10.2):
    /// - Created -> Validated | Failed
    /// - Validated -> Instantiated | Failed
    /// - Instantiated -> Running
    /// - Running -> Draining
    /// - Draining -> Completed | Cancelled | Failed
    ///
    /// # WARM PATH — called per flow state change.
    pub fn can_transition_to(&self, target: &FlowState) -> bool {
        matches!(
            (self, target),
            (FlowState::Created, FlowState::Validated)
            | (FlowState::Created, FlowState::Failed)
            | (FlowState::Validated, FlowState::Instantiated)
            | (FlowState::Validated, FlowState::Failed)
            | (FlowState::Instantiated, FlowState::Running)
            | (FlowState::Running, FlowState::Draining)
            | (FlowState::Draining, FlowState::Completed)
            | (FlowState::Draining, FlowState::Cancelled)
            | (FlowState::Draining, FlowState::Failed)
        )
    }

    /// Attempt to transition from `self` to `target`.
    ///
    /// Returns `Ok(target)` if the transition is legal, or
    /// `Err(InvalidTransition)` if it is not.
    ///
    /// # WARM PATH — called per flow state change.
    pub fn transition_to(self, target: FlowState) -> Result<FlowState, InvalidTransition> {
        if self.can_transition_to(&target) {
            Ok(target)
        } else {
            Err(InvalidTransition {
                machine: "FlowState",
                from: format!("{:?}", self),
                to: format!("{:?}", target),
            })
        }
    }

    /// Returns `true` if this state is terminal (no further transitions possible).
    ///
    /// # WARM PATH — checked for flow cleanup.
    #[inline]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            FlowState::Completed | FlowState::Cancelled | FlowState::Failed
        )
    }

    /// Returns `true` if this state is active (Created through Running).
    #[inline]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            FlowState::Created
                | FlowState::Validated
                | FlowState::Instantiated
                | FlowState::Running
                | FlowState::Draining
        )
    }
}

impl fmt::Display for FlowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowState::Created => write!(f, "Created"),
            FlowState::Validated => write!(f, "Validated"),
            FlowState::Instantiated => write!(f, "Instantiated"),
            FlowState::Running => write!(f, "Running"),
            FlowState::Draining => write!(f, "Draining"),
            FlowState::Completed => write!(f, "Completed"),
            FlowState::Cancelled => write!(f, "Cancelled"),
            FlowState::Failed => write!(f, "Failed"),
        }
    }
}

// ---------------------------------------------------------------------------
// ResourceState
// ---------------------------------------------------------------------------

/// Resource ownership state machine.
///
/// Per Doc 03, Section 3.1-3.2: tracks the lifecycle of host-managed resources
/// (primarily buffers). Extended here with `Transit` and `Freed` states.
///
/// State transition diagram:
/// ```text
///   Pooled -> Owned -> Borrowed -> Owned (borrow released)
///                   -> Leased   -> Owned (lease expired)
///                   -> Transit  -> Owned (new owner)
///                   -> Pooled   (released)
///   Any -> Freed (forced cleanup or shutdown)
/// ```
///
/// # Examples
/// ```
/// use torvyn_types::ResourceState;
///
/// let state = ResourceState::Pooled;
/// assert!(state.can_transition_to(&ResourceState::Owned));
/// assert!(!state.can_transition_to(&ResourceState::Borrowed));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ResourceState {
    /// The resource is in a buffer pool, not in active use.
    Pooled,
    /// The resource is exclusively owned by one entity.
    Owned,
    /// The resource is owned but has outstanding read-only borrows.
    Borrowed,
    /// The resource is held under a time- or scope-bounded lease.
    Leased,
    /// The resource is in transit between owners (host holds temporarily).
    Transit,
    /// The resource has been freed and its slot may be reused.
    Freed,
}

impl ResourceState {
    /// Returns `true` if transitioning from `self` to `target` is legal.
    ///
    /// Legal transitions (per Doc 03, Section 3.2, extended):
    /// - Pooled -> Owned (allocate)
    /// - Owned -> Borrowed (borrow started)
    /// - Owned -> Leased (lease granted)
    /// - Owned -> Transit (transfer initiated)
    /// - Owned -> Pooled (released to pool)
    /// - Owned -> Freed (deallocated)
    /// - Borrowed -> Owned (all borrows released)
    /// - Borrowed -> Borrowed (additional borrow — same state)
    /// - Leased -> Owned (lease expired/released)
    /// - Transit -> Owned (transfer completed to new owner)
    /// - Any -> Freed (forced cleanup: crash, shutdown)
    ///
    /// # HOT PATH — called per resource state change.
    pub fn can_transition_to(&self, target: &ResourceState) -> bool {
        // Any state can transition to Freed (forced cleanup)
        if *target == ResourceState::Freed {
            return true;
        }

        matches!(
            (self, target),
            (ResourceState::Pooled, ResourceState::Owned)
            | (ResourceState::Owned, ResourceState::Borrowed)
            | (ResourceState::Owned, ResourceState::Leased)
            | (ResourceState::Owned, ResourceState::Transit)
            | (ResourceState::Owned, ResourceState::Pooled)
            | (ResourceState::Owned, ResourceState::Freed)
            | (ResourceState::Borrowed, ResourceState::Owned)
            | (ResourceState::Borrowed, ResourceState::Borrowed) // additional borrow
            | (ResourceState::Leased, ResourceState::Owned)
            | (ResourceState::Transit, ResourceState::Owned)
        )
    }

    /// Attempt to transition from `self` to `target`.
    ///
    /// Returns `Ok(target)` if the transition is legal, or
    /// `Err(InvalidTransition)` if it is not.
    ///
    /// # HOT PATH — called per resource state change.
    pub fn transition_to(self, target: ResourceState) -> Result<ResourceState, InvalidTransition> {
        if self.can_transition_to(&target) {
            Ok(target)
        } else {
            Err(InvalidTransition {
                machine: "ResourceState",
                from: format!("{:?}", self),
                to: format!("{:?}", target),
            })
        }
    }

    /// Returns `true` if this state is terminal.
    #[inline]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, ResourceState::Freed)
    }

    /// Returns `true` if the resource is available for allocation from a pool.
    #[inline]
    pub const fn is_available(&self) -> bool {
        matches!(self, ResourceState::Pooled)
    }

    /// Returns `true` if the resource has an active owner.
    #[inline]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            ResourceState::Owned | ResourceState::Borrowed | ResourceState::Leased | ResourceState::Transit
        )
    }
}

impl fmt::Display for ResourceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceState::Pooled => write!(f, "Pooled"),
            ResourceState::Owned => write!(f, "Owned"),
            ResourceState::Borrowed => write!(f, "Borrowed"),
            ResourceState::Leased => write!(f, "Leased"),
            ResourceState::Transit => write!(f, "Transit"),
            ResourceState::Freed => write!(f, "Freed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // === FlowState valid transitions ===

    #[test]
    fn test_flow_state_created_to_validated() {
        assert!(FlowState::Created.can_transition_to(&FlowState::Validated));
        assert!(FlowState::Created.transition_to(FlowState::Validated).is_ok());
    }

    #[test]
    fn test_flow_state_created_to_failed() {
        assert!(FlowState::Created.can_transition_to(&FlowState::Failed));
    }

    #[test]
    fn test_flow_state_validated_to_instantiated() {
        assert!(FlowState::Validated.can_transition_to(&FlowState::Instantiated));
    }

    #[test]
    fn test_flow_state_validated_to_failed() {
        assert!(FlowState::Validated.can_transition_to(&FlowState::Failed));
    }

    #[test]
    fn test_flow_state_instantiated_to_running() {
        assert!(FlowState::Instantiated.can_transition_to(&FlowState::Running));
    }

    #[test]
    fn test_flow_state_running_to_draining() {
        assert!(FlowState::Running.can_transition_to(&FlowState::Draining));
    }

    #[test]
    fn test_flow_state_draining_to_completed() {
        assert!(FlowState::Draining.can_transition_to(&FlowState::Completed));
    }

    #[test]
    fn test_flow_state_draining_to_cancelled() {
        assert!(FlowState::Draining.can_transition_to(&FlowState::Cancelled));
    }

    #[test]
    fn test_flow_state_draining_to_failed() {
        assert!(FlowState::Draining.can_transition_to(&FlowState::Failed));
    }

    // === FlowState invalid transitions ===

    #[test]
    fn test_flow_state_created_to_running_invalid() {
        assert!(!FlowState::Created.can_transition_to(&FlowState::Running));
        let result = FlowState::Created.transition_to(FlowState::Running);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.machine, "FlowState");
        assert_eq!(err.from, "Created");
        assert_eq!(err.to, "Running");
    }

    #[test]
    fn test_flow_state_running_to_completed_invalid() {
        // Must go through Draining first
        assert!(!FlowState::Running.can_transition_to(&FlowState::Completed));
    }

    #[test]
    fn test_flow_state_completed_to_running_invalid() {
        // Terminal state — no transitions out
        assert!(!FlowState::Completed.can_transition_to(&FlowState::Running));
    }

    #[test]
    fn test_flow_state_failed_is_terminal() {
        assert!(FlowState::Failed.is_terminal());
        assert!(!FlowState::Failed.can_transition_to(&FlowState::Created));
        assert!(!FlowState::Failed.can_transition_to(&FlowState::Running));
    }

    #[test]
    fn test_flow_state_cancelled_is_terminal() {
        assert!(FlowState::Cancelled.is_terminal());
    }

    #[test]
    fn test_flow_state_instantiated_to_draining_invalid() {
        // Must go through Running first
        assert!(!FlowState::Instantiated.can_transition_to(&FlowState::Draining));
    }

    #[test]
    fn test_flow_state_is_active() {
        assert!(FlowState::Created.is_active());
        assert!(FlowState::Running.is_active());
        assert!(FlowState::Draining.is_active());
        assert!(!FlowState::Completed.is_active());
        assert!(!FlowState::Failed.is_active());
    }

    // === FlowState complete transition matrix ===

    #[test]
    fn test_flow_state_complete_valid_transition_count() {
        // There are exactly 9 valid transitions
        let states = [
            FlowState::Created, FlowState::Validated, FlowState::Instantiated,
            FlowState::Running, FlowState::Draining, FlowState::Completed,
            FlowState::Cancelled, FlowState::Failed,
        ];
        let mut valid_count = 0;
        for from in &states {
            for to in &states {
                if from.can_transition_to(to) {
                    valid_count += 1;
                }
            }
        }
        assert_eq!(valid_count, 9, "expected exactly 9 valid FlowState transitions");
    }

    // === ResourceState valid transitions ===

    #[test]
    fn test_resource_state_pooled_to_owned() {
        assert!(ResourceState::Pooled.can_transition_to(&ResourceState::Owned));
        assert!(ResourceState::Pooled.transition_to(ResourceState::Owned).is_ok());
    }

    #[test]
    fn test_resource_state_owned_to_borrowed() {
        assert!(ResourceState::Owned.can_transition_to(&ResourceState::Borrowed));
    }

    #[test]
    fn test_resource_state_owned_to_leased() {
        assert!(ResourceState::Owned.can_transition_to(&ResourceState::Leased));
    }

    #[test]
    fn test_resource_state_owned_to_transit() {
        assert!(ResourceState::Owned.can_transition_to(&ResourceState::Transit));
    }

    #[test]
    fn test_resource_state_owned_to_pooled() {
        assert!(ResourceState::Owned.can_transition_to(&ResourceState::Pooled));
    }

    #[test]
    fn test_resource_state_borrowed_to_owned() {
        assert!(ResourceState::Borrowed.can_transition_to(&ResourceState::Owned));
    }

    #[test]
    fn test_resource_state_borrowed_to_borrowed() {
        // Additional borrows stay in Borrowed state
        assert!(ResourceState::Borrowed.can_transition_to(&ResourceState::Borrowed));
    }

    #[test]
    fn test_resource_state_leased_to_owned() {
        assert!(ResourceState::Leased.can_transition_to(&ResourceState::Owned));
    }

    #[test]
    fn test_resource_state_transit_to_owned() {
        assert!(ResourceState::Transit.can_transition_to(&ResourceState::Owned));
    }

    #[test]
    fn test_resource_state_any_to_freed() {
        let states = [
            ResourceState::Pooled, ResourceState::Owned, ResourceState::Borrowed,
            ResourceState::Leased, ResourceState::Transit, ResourceState::Freed,
        ];
        for state in &states {
            assert!(
                state.can_transition_to(&ResourceState::Freed),
                "{:?} should be able to transition to Freed",
                state
            );
        }
    }

    // === ResourceState invalid transitions ===

    #[test]
    fn test_resource_state_pooled_to_borrowed_invalid() {
        assert!(!ResourceState::Pooled.can_transition_to(&ResourceState::Borrowed));
    }

    #[test]
    fn test_resource_state_pooled_to_leased_invalid() {
        assert!(!ResourceState::Pooled.can_transition_to(&ResourceState::Leased));
    }

    #[test]
    fn test_resource_state_borrowed_to_pooled_invalid() {
        // Must return to Owned first
        assert!(!ResourceState::Borrowed.can_transition_to(&ResourceState::Pooled));
    }

    #[test]
    fn test_resource_state_borrowed_to_transit_invalid() {
        // Cannot transfer while borrows outstanding
        assert!(!ResourceState::Borrowed.can_transition_to(&ResourceState::Transit));
    }

    #[test]
    fn test_resource_state_leased_to_pooled_invalid() {
        // Must return to Owned first
        assert!(!ResourceState::Leased.can_transition_to(&ResourceState::Pooled));
    }

    #[test]
    fn test_resource_state_transit_to_pooled_invalid() {
        // Must become Owned by new entity first
        assert!(!ResourceState::Transit.can_transition_to(&ResourceState::Pooled));
    }

    #[test]
    fn test_resource_state_freed_is_terminal() {
        assert!(ResourceState::Freed.is_terminal());
        // Freed can only go to Freed
        assert!(!ResourceState::Freed.can_transition_to(&ResourceState::Pooled));
        assert!(!ResourceState::Freed.can_transition_to(&ResourceState::Owned));
    }

    #[test]
    fn test_resource_state_is_available() {
        assert!(ResourceState::Pooled.is_available());
        assert!(!ResourceState::Owned.is_available());
        assert!(!ResourceState::Freed.is_available());
    }

    #[test]
    fn test_resource_state_is_active() {
        assert!(!ResourceState::Pooled.is_active());
        assert!(ResourceState::Owned.is_active());
        assert!(ResourceState::Borrowed.is_active());
        assert!(ResourceState::Leased.is_active());
        assert!(ResourceState::Transit.is_active());
        assert!(!ResourceState::Freed.is_active());
    }

    // === InvalidTransition ===

    #[test]
    fn test_invalid_transition_display_is_actionable() {
        let err = InvalidTransition {
            machine: "FlowState",
            from: "Running".into(),
            to: "Created".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("FlowState"));
        assert!(msg.contains("Running"));
        assert!(msg.contains("Created"));
        assert!(msg.contains("not permitted"));
    }
}
