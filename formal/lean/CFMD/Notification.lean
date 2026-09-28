/-
CFMD wake-only publication notification model.

A notification backend is deliberately outside database-state authority. It
may produce duplicate or spurious wakes, but only a durable publication can
advance the authoritative Revision. Watch correctness therefore depends on
recovering causal effects from durable history after wake, not on trusting the
notification payload.
-/

namespace CFMD.Notification

structure ObserverState where
  revision : Nat
  wakeGeneration : Nat
  deriving DecidableEq, Repr

/-- A backend wake changes liveness state only. -/
def notify (state : ObserverState) : ObserverState :=
  { state with wakeGeneration := state.wakeGeneration + 1 }

/-- A real durable publication advances Revision and may also wake observers. -/
def publish (targetRevision : Nat) (state : ObserverState) : ObserverState :=
  { revision := targetRevision, wakeGeneration := state.wakeGeneration + 1 }

/-- One arbitrary wake cannot manufacture authoritative database state. -/
theorem notify_preserves_revision (state : ObserverState) :
    (notify state).revision = state.revision := by
  rfl

def repeatNotify : Nat → ObserverState → ObserverState
  | 0, state => state
  | count + 1, state => repeatNotify count (notify state)

/-- Any finite number of duplicate/spurious wakes preserves Revision. -/
theorem repeated_notify_preserves_revision (count : Nat) (state : ObserverState) :
    (repeatNotify count state).revision = state.revision := by
  induction count generalizing state with
  | zero => rfl
  | succ count ih =>
      simp [repeatNotify, ih, notify]


/-- Watch-local lifecycle state is orthogonal to database Revision authority. -/
structure SubscriptionState where
  observer : ObserverState
  cancelled : Bool
  deriving DecidableEq, Repr

/-- Cancelling a watcher changes only watcher liveness and emits a wake. -/
def cancel (state : SubscriptionState) : SubscriptionState :=
  { observer := notify state.observer, cancelled := true }

/-- Runtime shutdown also acts only as a wake from the observer's perspective. -/
def shutdownWake (state : SubscriptionState) : SubscriptionState :=
  { state with observer := notify state.observer }

/-- Cancellation cannot manufacture a database Revision. -/
theorem cancel_preserves_revision (state : SubscriptionState) :
    (cancel state).observer.revision = state.observer.revision := by
  rfl

/-- A shutdown wake cannot manufacture a database Revision either. -/
theorem shutdown_wake_preserves_revision (state : SubscriptionState) :
    (shutdownWake state).observer.revision = state.observer.revision := by
  rfl

/-- Cancellation is sticky for the subscription itself. -/
theorem cancel_marks_subscription_closed (state : SubscriptionState) :
    (cancel state).cancelled = true := by
  rfl

/-- Protocol watch opening materializes observer state without changing database authority. -/
def protocolOpen (state : ObserverState) : SubscriptionState :=
  { observer := state, cancelled := false }

/-- Pulling the next protocol event is observational until a durable publication exists. -/
def protocolNext (state : SubscriptionState) : SubscriptionState := state

/-- Closing one hosted protocol subscription is cancellation, not publication. -/
def protocolClose (state : SubscriptionState) : SubscriptionState := cancel state

/-- Closing a hosted session cancels all server-side subscriptions. -/
def protocolCloseSession (states : List SubscriptionState) : List SubscriptionState :=
  states.map protocolClose

theorem protocol_open_preserves_revision (state : ObserverState) :
    (protocolOpen state).observer.revision = state.revision := by
  rfl

theorem protocol_next_preserves_revision (state : SubscriptionState) :
    (protocolNext state).observer.revision = state.observer.revision := by
  rfl

theorem protocol_close_preserves_revision (state : SubscriptionState) :
    (protocolClose state).observer.revision = state.observer.revision := by
  rfl

theorem protocol_close_session_preserves_revisions (states : List SubscriptionState) :
    (protocolCloseSession states).map (fun state => state.observer.revision) =
      states.map (fun state => state.observer.revision) := by
  simp [protocolCloseSession, protocolClose, cancel, notify]

/-- Publication authority is explicit and independent from wake history. -/
theorem publish_sets_exact_revision
    (targetRevision : Nat) (state : ObserverState) :
    (publish targetRevision state).revision = targetRevision := by
  rfl

end CFMD.Notification
