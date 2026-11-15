# Controlled Wallpaper Window Attachment, Native Window Styles & Transactional Rollback

**Module:** `crates/kinetiwall-desktop`  
**Phase:** Phase 7: Native Windows Desktop Hosting & Wallpaper Surface Integration  
**Step:** 40 of 90  
**Implementation Standard:** Safe Win32 FFI, Process-Isolated, Transactional Rollback, Strictly Validated

---

## 1. Architectural Overview

Hosting live wallpaper surfaces directly on the Windows desktop requires reparenting a dedicated player surface window into Windows Explorer's secondary background worker window (`WorkerW`) sitting directly behind the desktop icon view container (`SHELLDLL_DefView`). 

Because reparenting live windows across processes modifies Win32 message routing, coordinate contexts, and window hierarchies, **attachment must be treated as a reversible transaction**:
1. Save the original source window state prior to any mutation.
2. Validate source eligibility and destination compatibility.
3. Apply minimal required window mutations in strict sequence.
4. Verify the resulting relationship against native OS properties.
5. Guarantee deterministic rollback if any stage fails.

```
                           +----------------------------+
                           |   AttachmentCoordinator    |
                           +--------------+-------------+
                                          |
                +-------------------------+-------------------------+
                |                                                   |
                v                                                   v
   +---------------------------+                       +---------------------------+
   |  Original State Snapshot  |                       |     Transaction Engine    |
   | (Parent, Style, Bounds)   |                       | (Stages 0 - 5 with Undo)  |
   +---------------------------+                       +-------------+-------------+
                                                                     |
                                                                     v
                                                       +---------------------------+
                                                       |   DesktopWindowProvider   |
                                                       | (Native Win32 or Mock FFI)|
                                                       +-------------+-------------+
                                                                     |
                           +-----------------------------------------+
                           |
                           v
              +--------------------------+
              |   Target Host (WorkerW)  |
              |   +--------------------+ |
              |   | Wallpaper Player   | |
              |   | (WS_CHILD Surface) | |
              |   +--------------------+ |
              +--------------------------+
```

---

## 2. Attachment Eligibility & Safeguards

Before modifying any window property, `AttachmentCoordinator::validate_attachment_eligibility` enforces strict preconditions:
- **Null / Invalid Handle Rejection**: The handle must be non-zero and pass Win32 `IsWindow`.
- **Management UI Guard**: Handles registered as protected application windows (such as KinetiWall's management dashboard) are **strictly rejected**.
- **Desktop Root & Icon Host Guard**: Windows with class `"Progman"`, `"SHELLDLL_DefView"`, or `"SysListView32"` are permanently prohibited as sources.
- **Process Ownership**: The source window must belong to the expected player process ID (`owning_process_id`). Foreign windows cannot be hijacked.
- **Target Role Verification**: Target window must have role `CandidateRole::WallpaperHostCandidate` and must **not** contain `SHELLDLL_DefView`.
- **DPI Context Equivalence**: If both source and target windows report DPI awareness contexts via `GetWindowDpiAwarenessContext`, they must be equal (`AreDpiAwarenessContextsEqual`). Differing DPI contexts prevent clean Win32 parenting and trigger safe rejection.

---

## 3. Native Win32 API Implementation & Ordering

### 3.1 Relevant APIs
1. **`SetParent(hWndChild, hWndNewParent)`**:
   - Reassigns the parent window.
   - Clears prior parent. If `hWndNewParent` is `NULL`, the window becomes a top-level window.
   - Checked with `SetLastError(0)` prior to call.
2. **`SetWindowLongPtrW(hWnd, GWL_STYLE, dwNewLong)`**:
   - Updates standard window style bits.
   - Requires `WS_CHILD` to be set and `WS_POPUP` to be cleared for child windows.
   - Checked with `SetLastError(0)` prior to call.
3. **`SetWindowLongPtrW(hWnd, GWL_EXSTYLE, dwNewLong)`**:
   - Updates extended window style bits.
   - Adds `WS_EX_TOOLWINDOW` to eliminate taskbar buttons.
   - Adds `WS_EX_NOACTIVATE` to prevent the wallpaper surface from stealing focus or becoming the foreground window on mouse clicks.
4. **`SetWindowPos(hWnd, hWndInsertAfter, X, Y, cx, cy, uFlags)`**:
   - Notifies the window manager of non-client frame updates via `SWP_FRAMECHANGED`.
   - Positions and sizes the surface to match the target host bounds with `SWP_NOACTIVATE | SWP_SHOWWINDOW`.
5. **`GetWindowDpiAwarenessContext(hWnd)` & `AreDpiAwarenessContextsEqual(ctx1, ctx2)`**:
   - Queries OS-level DPI awareness context to ensure compatible scaling across window station boundaries.

### 3.2 Sequence of Operations
The Win32 documentation specifies that when reparenting a window via `SetParent`, window styles must be adapted to match the new relationship:
1. **Capture State**: Capture original parent, style, ex_style, bounds, and visibility.
2. **Apply Styles**:
   - `new_style = (original_style & !WS_POPUP) | WS_CHILD | WS_CLIPSIBLINGS;`
   - `new_ex_style = original_ex_style | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;`
   - Call `SetWindowLongPtrW(source, GWL_STYLE, new_style)`.
   - Call `SetWindowLongPtrW(source, GWL_EXSTYLE, new_ex_style)`.
   - Call `SetWindowPos(source, NULL, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED)`.
3. **Apply Parent**:
   - Call `SetParent(source, target)`.
4. **Apply Bounds**:
   - Call `SetWindowPos(source, HWND_TOP, 0, 0, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW)`.
5. **Verify**:
   - Check `GetParent(source) == target`.
   - Check `GetWindowLongPtrW(source, GWL_STYLE)` has `WS_CHILD` and not `WS_POPUP`.
   - Check `IsWindowVisible(source) == true`.
   - Check `GetWindowRect(source)` is non-empty.

---

## 4. Transaction Stages & Rollback Mechanics

### 4.1 Transaction Stages
The transaction progresses through sequential, numbered stages:
- `Stage 0: Validation` (No mutations)
- `Stage 1: StateCaptured` (Original state safely recorded in memory)
- `Stage 2: StylePrepared` (Window styles mutated; no parenting yet)
- `Stage 3: ParentAssigned` (`SetParent` succeeded)
- `Stage 4: BoundsApplied` (`SetWindowPos` dimensions applied)
- `Stage 5: Verified` (All post-mutation assertions passed)

### 4.2 Rollback Routine
If an error occurs at any point before commit, the coordinator aborts and executes `execute_restoration` targeting the exact mutations applied:
- If `Stage >= ParentAssigned`: Calls `SetParent(source, original_parent)` to unparent the window.
- If `Stage >= StylePrepared`: Restores `original_style` and `original_ex_style` via `SetWindowLongPtrW`, followed by `SWP_FRAMECHANGED`.
- If `Stage >= BoundsApplied`: Restores original bounding rectangle and visibility via `SetWindowPos`.
- **Verification**: Re-queries `GetParent` and `GWL_STYLE`. If restored properties match the original snapshot, returns `RollbackOutcome::RollbackCompleted { verified: true }`.

---

## 5. Controlled Detachment Lifecycle

Detachment is a first-class operational lifecycle transition:
- **Player Stop & Replacement**: Detaches the active window before player process shutdown.
- **Application Shutdown**: `detach_all_shutdown()` iterates through all tracked attachments and safely detaches each window.
- **Idempotency**: Repeated detach calls on an already detached window return a clean no-op without mutating OS window state.
- **Non-Destructive Teardown**: Releasing parentage restores the window to top-level status without abruptly killing processes or crashing Explorer.

---

## 6. Verification and Testing Matrix

The implementation is verified by two test suites in `crates/kinetiwall-desktop`:
1. **Synthetic Unit Tests (`tests/attachment_tests.rs`)**:
   - `test_successful_attachment_transaction_and_verification`: Verifies normal flow, style bit updates, and post-attachment verification.
   - `test_rejection_of_prohibited_management_ui_handle`: Verifies that UI dashboard handles are rejected.
   - `test_rejection_of_desktop_root_and_icon_carrier_as_source`: Rejects Progman and SHELLDLL_DefView.
   - `test_rejection_of_unrelated_process_ownership`: Rejects unauthorized PIDs.
   - `test_rejection_of_ineligible_target_carrier_window`: Rejects targets hosting desktop icons.
   - `test_rejection_of_dpi_incompatible_windows`: Rejects windows with conflicting DPI awareness contexts.
   - `test_rollback_on_set_parent_failure`: Simulates SetParent failure and proves style restoration.
   - `test_rollback_on_bounds_positioning_failure`: Simulates SetWindowPos failure and proves parent/style restoration.
   - `test_controlled_detachment_and_restoration_lifecycle`: Verifies complete detach and idempotency.
   - `test_shutdown_detaches_all_active_windows`: Verifies clean multi-window detachment on shutdown.
2. **Native Windows Integration Tests (`tests/native_integration_tests.rs`)**:
   - `test_native_window_style_and_lifecycle_contract`: Tests real Win32 `CreateWindowExW`, `SetWindowLongPtrW`, `GetWindowLongPtrW`, and `DestroyWindow`.
   - `test_native_attachment_coordinator_full_transaction`: Executes end-to-end attach, parenting, verification, and detach on live OS window handles.
   - `test_native_desktop_window_enumeration_validity`: Validates live top-level window enumeration in the active window station.

---

## 7. Scope Boundaries and Next Steps

- **Step 40 Boundary**: Single verified host window and single display surface attachment only. Multi-monitor management, monitor hotplugging, and coordinate spanning belong strictly to Step 41.
- **Non-Interference**: Desktop icons remain interactive and unoccluded. Explorer is never restarted or terminated.

