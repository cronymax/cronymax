## ADDED Requirements

### Requirement: Title bar is a native CEF Views panel

The system SHALL render a title bar as a `CefPanel` with a horizontal `CefBoxLayout`, mounted as the first child of the window's vertical root layout. The title bar SHALL have a fixed preferred height of 38 px. The title bar SHALL NOT be implemented as an HTML `BrowserView`.

#### Scenario: Native panel, not BrowserView

- **WHEN** the build links the application and the main window is constructed
- **THEN** `MainWindow::titlebar_panel_` exists, is a `CefPanel`, and contains exactly the children defined by this capability — no `BrowserView` is created for the title bar

#### Scenario: Spans full window width

- **WHEN** the main window is shown at any size
- **THEN** the title bar spans the full window width above both the sidebar and the content area

---

### Requirement: Title bar layout slots

The title bar SHALL contain, left to right: a 78 px traffic-light reservation pad, a flex spacer, three new-tab buttons, and a window-controls reservation pad. On macOS the window-controls pad SHALL be 0 px wide.

#### Scenario: Slot order

- **WHEN** the title bar is laid out
- **THEN** child views appear in the order `lights_pad_`, `spacer_`, `btn_web_`, `btn_term_`, `btn_chat_`, `win_pad_`

#### Scenario: Spacer absorbs free space

- **WHEN** the window is resized
- **THEN** only `spacer_` changes width; the buttons and reservation pads keep their preferred sizes

---

### Requirement: New-tab buttons

The title bar SHALL expose three `CefLabelButton` actions: "+ Web", "+ Terminal", "+ Chat". Each button SHALL have a tooltip set via `CefButton::SetTooltipText` ("New web tab", "New terminal", "New chat"). Clicking a button SHALL send `shell.tab_new_kind` with the corresponding kind.

#### Scenario: Tooltip on hover

- **WHEN** the user hovers any title-bar new-tab button for the OS tooltip delay
- **THEN** the platform tooltip appears with the button's configured text

#### Scenario: Click opens new tab

- **WHEN** the user clicks "+ Terminal"
- **THEN** the C++ side opens a new terminal tab and activates it; the sidebar shows a new row labeled `Terminal N`

---

### Requirement: shell.tab_new_kind channel

The renderer→C++ channel `shell.tab_new_kind` SHALL accept `{ kind: "web" | "terminal" | "chat" }` and respond with `{ tabId: string, kind: string }`. The dispatcher SHALL open a new tab of the requested kind, activate it, and broadcast `shell.tab_created`.

#### Scenario: Web kind uses default home

- **WHEN** the renderer sends `shell.tab_new_kind { kind: "web" }`
- **THEN** the C++ side opens a web tab navigating to the default home URL (currently `https://www.google.com`)

#### Scenario: Multiple terminals are independent

- **WHEN** the renderer sends `shell.tab_new_kind { kind: "terminal" }` three times in succession
- **THEN** three separate terminal tabs exist, each with its own tab id, named `Terminal 1`, `Terminal 2`, `Terminal 3`

---

### Requirement: macOS window dragging from spacer

On macOS, the title bar SHALL be draggable via mouse-down in the `spacer_` region. The traffic-light reservation pad and the new-tab buttons SHALL NOT be draggable (clicks pass to the OS lights or the button respectively).

#### Scenario: Drag from spacer moves the window

- **WHEN** the user mouse-downs in the title-bar spacer area and drags
- **THEN** the window moves with the cursor as if dragged by the OS title bar

#### Scenario: Click on button does not start a drag

- **WHEN** the user mouse-downs on "+ Web" and drags within the button hit-rect
- **THEN** the window does not move; the button receives the click

#### Scenario: Drag region tracks resize

- **WHEN** the window is resized
- **THEN** the AppKit drag overlay frame is updated to the new spacer rect on the next layout pass

---

### Requirement: Traffic-light reservation

On macOS the title bar SHALL leave a 78 px wide, 26 px tall area at the leftmost slot empty so the OS-drawn traffic lights remain visible and unoccluded. No CEF child view SHALL be placed within this area.

#### Scenario: Lights remain visible

- **WHEN** the window is shown
- **THEN** the macOS traffic lights are visible over the title bar with no CEF widget overlapping them

---

### Requirement: Windows-controls reservation

The title bar SHALL include a trailing window-controls slot. On macOS the slot SHALL be 0 px wide. The slot exists as a named insertion point so a future Windows port can fill it without rearranging the layout.

#### Scenario: macOS leaves slot empty

- **WHEN** the build links the application on macOS
- **THEN** `win_pad_` exists with preferred width 0

---

### Requirement: Popover layout uses title-bar height

`MainWindow::LayoutPopover` SHALL use the title-bar height (not the legacy topbar height) when computing the popover's vertical inset.

#### Scenario: Popover centres under the title bar

- **WHEN** the user opens a popover
- **THEN** the popover's top edge is positioned just below the title bar, not 44 px below the top of the window
