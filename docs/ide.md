# The window of the IDE (M1)

*Step 6 of the specifications of M1, decided with the owner on 2026-11-15. What the window shows, where, and what the
agent attached to it sees and does. The canvas and its gestures are in [visual-notation.md](visual-notation.md)
(sections 2.4 to 2.6), the editing commands in [commands.md](commands.md) (section 9), the server in
[mcp.md](mcp.md). A value marked (proposed) is settled when the Rust of M1 is written, after it has been tried in
the window.*

The window follows the public conventions of desktop IDEs and of the Craft apps; it borrows no look, icon or text from
NI (D-23). Its pieces that another IDE can use are written for that from the start (section 11).

## 1. Principles

- **Everything is a command.** A menu entry, a key, a button of a panel, the command palette and the agent call
  the same commands ([commands.md](commands.md)); the window draws their state and adds nothing that only a click
  can do.
- **The layout is the user's.** Where the panels are, their size, what is folded, which views are detached: the
  settings of the user, never those of the project, never the history. A project opened on another computer opens
  in that user's layout.
- **One project, one session, one history**, however many windows show it (section 9).
- **Nothing hidden.** Every gesture has its entry in a menu with its key (notation 2.6), every panel its entry in
  `View`, every command its line in the command palette.
- **What is planned is visible.** The milestones to come appear where they will be, greyed, marked Planned, each
  leading to its milestone or its issue on GitHub, with no date promised (section 6).
- **Readable at a glance.** The diagram keeps its colours (notation 8): whatever draws the panels, the canvas takes
  its colours from the table of tokens of the notation.

## 2. The window

```text
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│ logo  File Edit View Run Help  ↶ ↷ │ Edit ▾ │   project — search or run a command   │ agent ⚙ ? │
├───────────┬───────────────────────────────────────────────────────────────┬─────────────┤
│ Project   │ check_voltage × │ read_voltage ×        Diagram Text Block Panel │ Inspector   │
│ Palette   │                                                                 │             │
│           │                         canvas                                  │             │
│           ├───────────────────────────────────────────────────────────────┤             │
│           │ Problems │ Run                                                   │             │
├───────────┴───────────────────────────────────────────────────────────────┴─────────────┤
│ file · saved │ last step and its author                      agent │ color by type │ zoom │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

- **The top bar is the title bar**: the logo; a house that opens the start page; the menus `File`, `Edit`, `View`, `Run`, `Help`; undo and redo; the
  workspace selector (section 6); in the middle, the name of the project and the field that opens the command palette
  (section 5); on the right, the state of the agent (section 8), the settings, the help, the version and, where the
  desktop draws none, the buttons of the window. An empty part of the bar moves the window, a double click
  maximizes it. On macOS the menus are those of the system bar, and the bar leaves room for the buttons of the
  system; elsewhere they open from the bar. One description of the menus, as data, gives both (`craft-menu`), and
  `craft-window` decides who draws the frame.
- **Left: what the user takes.** `Project` (the tree of the folder) and `Palette` (the functions to place), two tabs
  of one dock.
- **Centre: the tabs of the functions**, each with its views (section 3).
- **Right: what is selected.** `Inspector` (section 4).
- **Bottom: what happens.** `Problems` and `Run`.
- **The status bar**: the file shown and whether it is written; the last step of the history and its author
  ([commands.md](commands.md) 9.1); in the `Text` view, the line and the column; the state of the agent; color by type
  (notation 4.3); the zoom.

Every panel can be moved, docked elsewhere, folded or closed (`craft-ui`, docking); `View > Reset layout` brings back
the layout above. The window opens at 1,280 × 800 at least *(proposed)*; under 1,000 pixels of width, the side
panels fold first.

## 3. The tabs of a function

A tab shows one function. A double click on a call, on a file of `Project`, or `Open` in the command palette opens it.
A single click on a file opens it as a **preview**: its title in italics, replaced by the next preview, kept when the
user edits it or double-clicks its title. Preview tabs are a setting of the user, on by default.

The **views** of a tab are the parts of its file, each its own way:

| View | Shows and edits | M1 |
|---|---|---|
| `Diagram` | The body of the function and its `layout` (the canvas, notation 2) | Yes, the default |
| `Text` | The whole file (section 7) | Yes |
| `Block` | What a caller sees: the `terminals`, `outline` and `icon` sections and the band (spec 11.1 to 11.4) | A live preview; terminals dragged along their side; the template and the band chosen from a list. Drawing the icon by hand comes later |
| `Panel` | The front panel of the function (D-06) | Planned (M3) |

**Split** shows two views of one function side by side, each with its own scroll: `Text` and `Diagram`, `Text` and
`Block`. A change in one is in the other as soon as the text reads (commands.md 9.1). `Text` with `Block` is how a
developer writes an icon or an outline in M1: the block redraws at real size and enlarged while they type, with the
warnings of `check` under the text.

The view of each tab, and its split, are view state (spec 11): kept for the user, never in the file.

## 4. The panels

- **`Project`**: the tree of the folder, the files of each library under it; a function that does not draw shows
  the number of its errors; the shipped libraries (`core`, `visa`, `scpi`…), read only, at the end. A file opened
  in a detached window says so. Right click: new function, new type, rename, delete, open in a window, each a
  command.
- **`Palette`**: the functions the project can call, grouped by library, with a search field on top that searches as
  quick insertion does (notation 2.6). A function dragged onto the canvas is a node added there (`node.add`).
- **`Inspector`**: what the selection is (decision of the owner, 2026-11-14):
  - one node: its function, its signature, its documentation, the help of each parameter, its values of the last run,
    whether it is fixed, with "Remove the fixed position";
  - one wire: its type, its source and its destination, its value of the last run, its `via` line;
  - one terminal of the frame: its name, its type, its default, whether it is fixed;
  - nothing: the function itself, with its signature, its documentation, its icon and its `frame size`;
  - several: how many, and the actions they share.

  A field changed in the inspector calls the command of the same gesture, and is one step of the history.
- **`Problems`**: the diagnostics of the project, by file, error first; a click opens the function and selects the
  node, or the line in the `Text` view. A filter by severity, and one for the function shown.
- **`Run`**: the run of the function shown, its status, its report and its trace (the bar of the viewer of M0v, as a
  panel), and the choice of the simulation file.

## 5. The command palette

Ctrl+Shift+P, or F1, or a click on the field of the top bar (notation 2.6). It lists every command of the IDE, by its
label, with its key, searched as quick insertion searches; Enter runs it, and a command that needs parameters opens
them in a small form. Typed as an id followed by a JSON object, `node.add {"call": "core.ge"}`, it runs that command
with those parameters, and a badge says whether the object is valid before Enter. The commands that the user cannot
run now are listed, greyed, with the reason; the planned entries (section 6) are listed too.

## 6. Workspaces and what is planned

The **workspace selector** of the top bar holds the ways of working on the same project, history and selection;
changing it changes the panels and the views offered, nothing else. Each entry has a name, a state, one line on what
it does and, when it is planned, a link:

| Workspace | State in M1 | What it is |
|---|---|---|
| `Edit` | Current | Draw, wire and run functions; text and block side by side |
| `Panel` | Planned (M3) | The front panel: controls and indicators of a function |
| `Debug` | Planned (M5) | Probes, breakpoints, step by step on the diagram |
| `Build` | Planned (M6) | A standalone executable, a library with a C ABI, a .NET facade |

Planned entries are also where they will be: `Run > Real instruments` (M4), the `Panel` view of a tab (M3), the
icon drawn by hand in `Block`. A planned entry can never be chosen: a click opens a short card that says what it will
be and links to its milestone or its issue, in the browser. The entries are **data**, a list shipped with the IDE
(id, label, line, state, milestone, link), not code per entry; an entry becomes Available in the release that tags
its milestone.

## 7. The text of a function

The `Text` view (the `Text` panel of commands.md 9.1 and of notation 2.6) edits the file as the developer writes it; when the text is written, and what happens to a text that
does not read, is in [commands.md](commands.md) 9.1. In M1 it has: the colours of the tokens of the language, from the
lexer of LabCraft itself, so that it never disagrees with `check`; the diagnostics underlined, with their message on
hover and in `Problems`; completion of names (functions, values, types, keywords of the section it is in) in a popup,
opened by typing or by Ctrl+Space, taken with Enter or Tab; one cursor; the keys of a text field (notation 2.6).
Multiple cursors, folding and search come later.

It is a component of its own (section 11): a core with no egui (the text, the selection, the undo of the field,
and two traits, one that colours a line and one, the language service, that gives diagnostics, completions, hover and
definitions, in the types of `lsp-types`), and a thin layer of egui that paints it. Another IDE gives it its own
lexer and its own language service. Not chosen: the editor of VS Code (Monaco) or CodeMirror over the canvas, which
would add a second renderer, with its own focus, keys, fonts and popups, a web view on the desktop, and the
same work again in each IDE; tree-sitter, which would need a second grammar of the language beside the parser.

## 8. The agent attached to the IDE

The channel is that of [mcp.md](mcp.md) 9: on `127.0.0.1` only, a token for each launch in a file only the user can
read, a file that gives the port, so that `labcraft-cli mcp` finds the open window. It is open while the window is,
unless the user turns it off. With several projects open, each has its channel. `labcraft-cli mcp` attaches to the
window whose project is its project, `--project DIR` or the current folder (mcp.md 1); when no window shows that
project, it starts headless, as in M0b, so that an agent never writes into a project other than the one it was started
on (decision of the owner, 2026-11-15).

- **The first connection of a client asks** (the one exception to D-18, decided by the owner on 2026-11-15). The
  window names the client and the project, and offers three policies, by the `effects` of each command
  (commands.md 1): **Ask** (a command with no effect runs; one with `files` or `run` waits for the user's answer),
  **Edits** (`files` runs; `run` asks), **All** (`files` and `run` run, on the simulator). `history.undo` and
  `history.redo` count as `files`. A question waits 60 s *(proposed)*; with no answer, the command is
  `agent.not_allowed`, and so is a connection refused or not answered. No policy reaches a real instrument: how the user allows one is specified with M4 (mcp.md 6). The answer is
  kept, if the user wants, for that client and that project, in the settings of the user, never of the project.
  Refused, or not allowed by its policy, a command of the agent is `agent.not_allowed` (commands.md 3).
- **Its policy is the user's.** The settings of AI and of the agents (the switch, each policy) are refused to an
  agent, whatever its policy (`agent.not_allowed`); `settings.set` of another key is an edit.
- **What it sees.** Everything a command reads, and the state of the window: the function shown in each window, the
  view of each tab, the selection (`view.get_state`), so that "this node" means something. It never changes what the user
  sees: it does not select, scroll, zoom or switch tabs, and the `view.*` commands that would are refused to it
  (mcp.md 2).
- **What the user sees of it.** The pill of the top bar: no agent, or which one is connected, and its policy; a click
  disconnects it or changes its policy. Its edits go into the one history, signed with its name (commands.md 9.1,
  9.5); the nodes and wires it touched are highlighted for a moment; an edit in a function that is not shown puts a
  dot on its tab; a short notice says what it did, with Show and Undo.
- **One switch for AI**, in the settings, on by default: off, it closes the channel and takes every trace of AI out of
  the window, the pill and the notices included.
- **Connect an AI agent**, in `Help` and on the start page, writes the entry of the server in the configuration of
  the MCP clients found on the computer, changing nothing else in those files (`mcp setup`, mcp.md 9), and says
  which.
- **Its files.** An agent reads and writes only the project of its window, as `labcraft-cli mcp --project DIR`.

A chat panel inside the window, and a visible pointer of the agent, are left for later (section 13).

## 9. Windows

- **One project per window.** `Open folder` and `New project` from a window that shows a project open another window.
  The windows of one project share its session.
- **Detached windows** (decision of the owner, 2026-11-15). A tab of a function can be detached into a window of the
  system, and docked back. That window has the views of its function, its own canvas and its own inspector, for its
  own selection; the menus stay in the window of the project, which keeps `Project`, `Palette`, `Problems` and `Run`.
  Ctrl+Z, in any window of the project, undoes the last step of the project. A layout where the window of the
  project holds only the panels and every function has its own window is a layout like the others, kept by the user.
- On the web there is one window of the browser: a detached tab becomes a panel floating in it.

## 10. Start, new and open

- **The start page** opens when the IDE starts with no folder, and from the house of the top bar: the recent projects
  (12 at most, *(proposed)*, a project that is no longer there marked "not found", the check made off the window
  thread), `New project`, `Open folder`, the examples, and the settings, the keys and `Connect an AI agent`.
- **`New project`** asks for a folder, makes it with a function `main` (`main.lcf`, `fn main() -> () {}`), and opens
  it with `main` shown. The root of a project needs no manifest (spec 1).
- **`Open folder`** opens any folder of `.lcf` files: the project is the folder (mcp.md 1).
- **An example** opens as a copy, in a folder the user chooses, never the files shipped with LabCraft.

## 11. Pieces for other IDEs

LabCraft will not be the only IDE of its kind. The pieces of the window that are not about LabCraft are crates of
their own in this repository, which depend on no crate of LabCraft (a rule that `cargo xtask layers` gains with
them), with their own
documentation, MIT OR Apache-2.0; when they are stable, they are offered to `craft-libs`:

- the text component (section 7): `kit-text`, with no UI toolkit, and `kit-text-egui`, which paints it;
- the shell of the window, `kit-shell`: the top bar, the workspace selector with its planned entries, the start page,
  the command palette and its JSON form, the inspector as a list of fields;
- the docking and the detached windows come from `craft-ui` and `craft-window`; the keys from `craft-keymap`, the
  menus from `craft-menu`, the settings from `craft-settings`.

The canvas stays LabCraft's (`editor`), embeddable in another Craft app (D-27). Whether these pieces become a UI kit of
their own is decided when a second IDE starts.

## 12. The web

The same window, written once for the desktop and the browser, replaces the two panels of the viewer of M0v. In M1 the
web page edits a project held in memory (the examples, or a folder dropped on it), which `Download` saves as a zip;
it has no agent attached, and a detached tab floats in the page (9). The settings of the user are kept in the
`localStorage` of the page, those of the project in its `.labcraft/config.toml`, in memory and in the zip. Opening a
folder of the computer to write it in place (the File System Access API, which only Chromium has) is not in M1
([architecture.md](architecture.md), 12.9).

## 13. Left for later

- Drawing the icon by hand in `Block`, with a throwaway HTML prototype first.
- A chat with the agent inside the window, and a visible pointer of the agent that acts out its commands.
- Multiple cursors, folding and search in the text; a language server for the editors of others, built on the same
  language service.
- Named layouts, and a layout per workspace.
