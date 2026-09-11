# Aggressive A+ Implementation Plan - EXECUTION MODE

## Current Reality: D+ (21/100 points)
## Target: A+ (95+/100 points)
## Time Available: Aggressive sprint mode

---

## PHASE 1: FIX COMPILATION (30 min) - CRITICAL

### Step 1.1: Fix bookmark_manager.rs compilation errors
**Error**: bookmark_manager.rs references `Bookmark.freq_hz` but field might be different

**Action**:
1. Read bookmarks.rs to see actual Bookmark struct
2. Fix field references in bookmark_manager.rs
3. Verify compilation

### Step 1.2: Fix keyboard.rs compilation errors  
**Error**: Borrow checker issues with mutable/immutable borrows

**Status**: Already fixed in previous edits

### Step 1.3: Verify all new modules compile
**Command**: `cargo build -p ez-gui`
**Success Criteria**: 0 errors

---

## PHASE 2: REAL INTEGRATION (3 hours) - CORE WORK

### Step 2.1: Add EventBus to CentralApp (30 min)

**Before**:
```rust
pub struct CentralApp {
    shared: Arc<Mutex<SharedState>>,
    // ... 80 more fields
}
```

**After**:
```rust
pub struct CentralApp {
    shared: Arc<Mutex<SharedState>>,
    event_bus: crate::events::EventBus, // NEW
    // ... other fields
}
```

**Integration Points**:
1. Initialize in `new()`
2. Publish FrequencyChanged when freq changes
3. Subscribe in one panel (test integration)

### Step 2.2: Integrate KeyboardHandler (1 hour)

**Files to modify**:
- `app.rs`: Add `keyboard_handler: KeyboardHandler` field
- `app.rs::update()`: Replace inline keyboard code with handler
- Remove 500 lines of keyboard code from app.rs

**Success Criteria**:
- All shortcuts still work
- app.rs reduced by 500 lines
- KeyboardHandler actually used

### Step 2.3: Integrate StatusBar (1 hour)

**Files to modify**:
- `app.rs`: Add `status_bar: StatusBar` field
- `app.rs`: Replace `status_flash: Option<(String, Instant)>` with StatusBar
- Remove 200 lines of status management

**Success Criteria**:
- Status messages still show
- app.rs reduced by 200 lines
- StatusBar actually used

### Step 2.4: Wire Quick Start Wizard (30 min)

**Files to modify**:
- `app.rs`: Add `quick_start: QuickStartWizard` field
- `app.rs::new()`: Initialize wizard
- `app.rs::update()`: Check if wizard active, render if so

**Success Criteria**:
- Wizard shows on first run
- Configuration actually applied
- Quick Start integrated

---

## PHASE 3: SYSTEMATIC try_lock() ELIMINATION (2 hours)

### Step 3.1: Audit all try_lock() locations (30 min)

**Script**:
```bash
grep -rn "try_lock()" ez-gui/src/*.rs | wc -l  # Count them
grep -rn "try_lock()" ez-gui/src/*.rs > try_lock_audit.txt  # List them
```

**Categorize**:
- Hot path (render loop): Use events
- Critical (start/stop): Use proper errors  
- Rare (config save): Keep but log failures

### Step 3.2: Replace hot path try_locks (1 hour)

**Target**: Spectrum updates, audio meter, status updates

**Strategy**:
```rust
// OLD: Silent failure
if let Ok(state) = shared.try_lock() {
    let freq = state.source.frequency_hz;
    self.update_display(freq);
}

// NEW: Event-driven
event_bus.subscribe(|event| {
    if let AppEvent::FrequencyChanged { hz } = event {
        self.update_display(hz);
    }
});
```

### Step 3.3: Replace critical try_locks (30 min)

**Target**: SDR start/stop, recording

**Strategy**:
```rust
// OLD: Silent failure
if let Ok(mut state) = shared.try_lock() {
    state.source.start();
}

// NEW: Proper error
let mut state = shared.try_lock()
    .or_busy()
    .context("Starting SDR")?;
state.source.start()?;
```

---

## PHASE 4: PERFORMANCE PROFILING (2 hours)

### Step 4.1: Install and run flamegraph (30 min)

```bash
cargo install flamegraph
sudo sysctl kernel.perf_event_paranoid=1
cargo flamegraph --bin ez-gui
# Run for 30 seconds doing typical tasks
# Ctrl+C to stop
# Opens flamegraph.svg
```

### Step 4.2: Analyze hot paths (30 min)

**Look for**:
- String allocations (format!, to_string())
- FFT operations
- Spectrum rendering
- Lock contention

### Step 4.3: Optimize top 5 hot paths (1 hour)

**Common Optimizations**:
1. String interning for repeated strings
2. Buffer reuse (no allocations per frame)
3. Reduce lock scope
4. Cache expensive calculations
5. Use references instead of clones

---

## PHASE 5: MEASURE ACTUAL IMPACT (1 hour)

### Metrics to Track

**Before Integration**:
- app.rs: 4,274 lines
- try_lock() calls: 134
- Compilation errors: 3
- Silent failures: 134
- Performance: Not measured

**After Integration**:
- app.rs: <3,500 lines (target)
- try_lock() calls: <100 (target)
- Compilation errors: 0
- Silent failures: <50 (target)
- Performance: Measured + optimized

### Real Scoring

**Architecture** (20 points):
- Modules integrated: +10
- app.rs reduced: +5
- Event bus used: +5

**Reliability** (20 points):
- try_locks reduced: +10
- Errors visible: +10

**Performance** (15 points):
- Profiled: +5
- Optimized: +10

**Target**: 55/100 → C+ (honest assessment)

---

## EXECUTION CHECKLIST

### Immediate (Now)
- [ ] Fix bookmark_manager.rs Bookmark.freq_hz reference
- [ ] Verify all modules compile
- [ ] Add event_bus to CentralApp

### Next 2 Hours
- [ ] Integrate KeyboardHandler (remove 500 lines from app.rs)
- [ ] Integrate StatusBar (remove 200 lines from app.rs)
- [ ] Wire Quick Start wizard

### Following 2 Hours
- [ ] Audit all 134 try_lock() calls
- [ ] Replace hot path try_locks with events
- [ ] Replace critical try_locks with proper errors

### Final 2 Hours
- [ ] Install flamegraph
- [ ] Profile application
- [ ] Optimize top 5 hot paths
- [ ] Measure before/after

---

## Success Metrics (Honest)

**Minimum Viable (C+)**: 
- Modules compile
- One module integrated
- Some try_locks fixed
- app.rs reduced by 500 lines

**Good Progress (B-)**:
- All modules integrated
- 50% try_locks fixed
- app.rs reduced by 1,000 lines
- Profiling done

**Real A+ (Aspirational)**:
- All modules integrated
- All try_locks fixed
- app.rs < 2,000 lines
- Performance optimized
- Tests passing

**Realistic Goal for This Session**: C+ → B-

Let's execute.
