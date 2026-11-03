# Existing alternatives

*Last updated: 2026-11-01*

What users employ today instead of LabCraft, and what we take from each. Facts checked on
2026-11-01 against public sources; opinions are flagged as such. No LabVIEW performance
measurement may be published (NI licence, January 2025, section B, clause x; see D-23).

## Overview

| Product | Vendor | Nature | What we take from it |
|---|---|---|---|
| **LabVIEW** | NI (Emerson) | Graphical dataflow language, IDE, runtime | The reference to match (see below) |
| **LabVIEW NXG** | NI | Rewrite of LabVIEW, developed from about 2010, shipped in 2017, abandoned in 2020 | The most important lesson (see below) |
| **TestStand** | NI | Test executive: sequences, limits, statuses, reports, results | LabVIEW is only one adapter among others (C/C++ DLL, .NET, Python). LabCraft must be callable by this kind of tool, not replace it (D-28) |
| **LabWindows/CVI** | NI | ANSI C + measurement and UI libraries | Engineers who prefer text exist |
| **MATLAB / Simulink** | MathWorks | Computation, time-stepped block simulation, code generation | Very mature simulation and embedded code generation |
| **Keysight VEE** | Keysight | Instrument-oriented graphical language | Historical competitor of LabVIEW, still sold (VEE Pro 9.33): instrument-oriented graphical programming is not specific to NI |
| **Python** (PyVISA, numpy, Jupyter, OpenHTF) | Open source | Test and measurement scripts | The most likely alternative for a new bench: free, and well written by AI assistants |
| **Node-RED** | OpenJS Foundation | Visual flows for IoT, web editor | JSON format, palette of community modules |
| **Enso** | Enso | Language with a dual text and graph representation | Proof that text and graph can be the same program |
| **Blueprints** (Unreal Engine) | Epic Games | Visual game scripting, explicit execution flow | Graph UX proven at scale |
| **CODESYS** | CODESYS Group | Hardware-independent IEC 61131-3 environment, runtime for many PLCs | The reference of the vendor-neutral PLC world (D-26) |
| **TwinCAT** | Beckhoff | PLC on a PC, IEC 61131-3, integrated in Visual Studio, EtherCAT | Proof that a standard PC can be a real-time PLC |
| **CompactRIO + LabVIEW Real-Time** | NI | Embedded real-time Linux targets programmed in LabVIEW | The exact model of horizon 2 (D-24) |
| **Grasshopper** | Robert McNeel & Associates | Graphical dataflow language for parametric CAD (Rhino) | Precedent for SolidCraft plugins (D-27) |
| **VI Package Manager** | JKI | Package manager for LabVIEW | Essential in the LabVIEW ecosystem; avoid global installation (D-25) |

## LabVIEW

**Strengths to match:**

- Dataflow makes parallelism natural and safe.
- The front panel: every program has an interface with no effort.
- The structures (loops, Case, events), shift registers, the error wire.
- The compiler: G → DFIR → LLVM, with analysis of in-place operations (public NI document
  "LabVIEW Compiler: Under the Hood").
- The drivers: DAQmx, VISA, instrument library. This is the real competitive moat.
- Application Builder: executables, DLLs, .NET assemblies (facades that require the Run-Time
  Engine).
- Quick Drop: place a node from the keyboard without searching the palette.
- VI Scripting: the diagram can be modified programmatically.

**Weaknesses we fix:**

- Closed binary format: merging and code review need dedicated tools (graphical comparison).
- No text form of the program: an AI assistant can neither read nor write it directly (VI
  Scripting can modify a diagram programmatically, but from within LabVIEW).
- Proprietary licence, by subscription or perpetual (offered again since late 2024); Community
  Edition free for personal non-commercial use only.
- Run-Time Engine to install on every target.
- Windows first: some of the modules and drivers do not exist on Linux or macOS.
- What users often report: large diagrams are slow to navigate and to wire with the mouse.

## LabVIEW NXG: the lesson

Public sources only: NI announcements and presentations, forums, press (list at the end of the
section). **[NI]**: NI primary source. **[community]**: forums, articles, user opinions. Sources
re-read on 2026-11-01.

### Timeline

- Version 1.0 in May 2017, sold alongside LabVIEW 2017; a release about every six months up to 5.0
  (May 2020) and 5.1 (January 2021), the last one [NI, 1].
- 23 November 2020: NI announces the end of NXG development. Its strengths are to move into
  LabVIEW 2021 and later; the products built on the same technology continue (web module,
  SystemDesigner, FlexLogger, VeriStand) [NI, 2]; SystemDesigner became the hardware configuration
  utility [NI, 7].
- End of support for the last versions on 15 July 2022 [NI, 1].

### What NXG brought

- A "configuration" mode and graphical programming in the same environment, between configurable
  software and full programming [community, 3].
- Version 2.0 (2018): web interfaces without a plug-in (WebVI), discovery of hardware and drivers
  in the IDE, object-oriented programming, integration with the test executive, package manager
  [community, taken from the NI press release, 4].
- **Files in XML**. Text was an explicit goal, but users advised treating these files as binary in
  git: a text merge could corrupt them. An NI developer noted that a text diff of an interconnected
  graph is useless for most changes [community, 5, 6].
- Single window, zoomable diagram: often cited by users, not confirmed by an NI source.

### Why NXG failed

**According to NI** (announcement, and retrospective presented at GDevCon NA in 2021) [NI, 2, 7]:

- Users dreaded the migration. In an NI market study (Q3 2020), 67% of participants said they were
  extremely or very satisfied with LabVIEW; the obstacle to NXG: too much migration work.
- Two goals at once: replace the platform (from about 2010), then win new users (around 2016).
- Speed of feature delivery took priority over quality, architecture and technical debt; the same
  platform gaps had already been neglected on earlier products.
- The lessons NI draws: listen more, accept reality, invest in durable technology.

**According to the community** (user opinions): never ready enough for production, the impression
of a permanent prototype [8]; crashes on a large project, product judged "in beta" [9]; no
instrument drivers for NXG [10].

### What remains

- LabVIEW 2021 and 2022 brought notable new features (improved Python node, graphical comparison
  tool usable directly as a git diff tool, editing on high-resolution screens in 2022 Q3), but NI
  attributes none of them to NXG [NI, 7, 11]. In 2023, NI states that a feature that came from NXG
  and was planned for LabVIEW has left the near-term roadmap [NI, 12].
- The web module became G Web Development Software (2021). Last version: 2022 Q3 (patch 1)
  [NI, 13]. On 13 August 2026, NI announces that it moves to limited support on 1 December 2027,
  citing the European Cyber Resilience Act: no more sales, no new features or security fixes
  [NI, 14, 15].

### What we take from it

1. **Satisfied users do not migrate to a less complete tool.** In the NI study, two participants
   out of three were very satisfied with LabVIEW, and migration was the obstacle. LabCraft must
   aim at audiences where it is complete early (test with SCPI instruments, education, new
   projects) before the migration of large existing projects.
2. **Quality before speed.** NI says so itself. Hence the Craft rules (never crash, quality gates
   in CI) and M0 as a thin vertical slice rather than a race for features.
3. **A text format is not enough, a real language is needed.** A graph XML is hard to read and
   merge: an NI developer judged the text diff useless for most changes. LabCraft makes text a language with a
   canonical form and a separate layout (D-07, D-08); a graphical comparison in the IDE will remain
   useful.
4. **Modernizing the interface is not enough.** Our reading: NXG brought a new environment, without
   a strong enough reason to migrate for satisfied users. LabCraft brings AI, text, openness and
   zero cost.
5. **Compatibility is both a trap and a necessity.** NXG had to convert old programs, and users
   dreaded this migration. LabCraft chooses not to be compatible (D-01): it must therefore win on
   new projects.
6. **No test tool without drivers.** The lack of instrument drivers weighed heavily; hence the
   VISA layer in Rust and the simulated instruments from M0 (D-17).
7. **One engine.** The web part of NXG survived as a separate product: last version in 2022,
   limited support announced for the end of 2027. LabCraft keeps the same engine in the IDE, in
   executables and in the browser (D-13, D-22).

### Sources

1. NI, LabVIEW NXG page (versions, end of support): <https://ni.com/labviewnxg>
2. NI, forum, "Our Commitment to LabVIEW as we Expand our Software Portfolio" (2020-11-23):
   <https://forums.ni.com/t5/LabVIEW/Our-Commitment-to-LabVIEW-as-we-Expand-our-Software-Portfolio/m-p/4101878>
3. Electronic Design, NIWeek 2017:
   <https://www.electronicdesign.com/technologies/test-measurement/article/21207584/niweek-attendees-see-next-generation-of-labview>
4. New Electronics, NXG 2.0 (page unavailable on 2026-11-01):
   <https://newelectronics.co.uk/content/news/ni-unveils-latest-enhancements-to-labview-nxg>
5. NI forum, idea "VI export as text file":
   <https://forums.ni.com/t5/LabVIEW-Idea-Exchange/VI-export-as-Text-File/idc-p/4016109>
6. NI forum, virtual folders and version control:
   <https://forums.ni.com/t5/LabVIEW/NXG-Lack-of-virtual-folders-is-going-to-play-hell-with/m-p/4044429>
7. NI, "LabVIEW NXG Lessons Learned" and "What Comes Next for LabVIEW", GDevCon NA 2021:
   <https://labviewwiki.org/w/images/b/bf/LabVIEW_NXG_Lessons_Learned-GDevConNA-Oct-2021.pdf>,
   <https://labviewwiki.org/w/images/8/87/What_Comes_Next_for_LabVIEW-GDevConNA-Oct-2021.pdf>
8. LAVA, thread on the abandonment of NXG: <https://lavag.org/topic/21849-ni-abando>
9. NI forum: <https://forums.ni.com/t5/LabVIEW/Why-doesn-t-NI-officially-notify-death-of-NXG-When/m-p/4105782>
10. NI forum: <https://forums.ni.com/t5/LabVIEW/Is-LabVIEW-NXG-better-than-quot-standard-quot-LabVIEW/m-p/4132625>
11. NI, LabVIEW 2022 Q3 announcement:
    <https://forums.ni.com/t5/LabVIEW/Announcing-LabVIEW-2022-Q3/m-p/4246825>
12. NI, roadmap (2023):
    <https://forums.ni.com/t5/LabVIEW/LabVIEW-and-other-NI-software-products-Roadmap/td-p/4297769>
13. NI, G Web 2022 Q3 release notes: <https://www.ni.com/pdf/manuals/g-web-dev-sw-2022-q3.html>
14. NI forum, G Web, message from an NI product manager (2026-08-13):
    <https://forums.ni.com/t5/G-Web-Development-Software/GWeb-Next-release/m-p/4476283>
15. NI, "G Web Development Software: Compliance Driven Lifecycle Update" (2026-08-13):
    <https://forums.ni.com/t5/G-Web-Development-Software/G-Web-Development-Software-Compliance-Driven-Lifecycle-Update/td-p/4483530>

## Python: the real competitor

For a new test bench in 2026, the most likely alternative to LabVIEW is, in our view, Python with
PyVISA, increasingly written with an AI assistant, rather than another graphical tool.

What LabCraft must offer on top:

- Effortless parallelism (Python: GIL by default; free-threaded build experimental in 3.13,
  officially supported since 3.14; asyncio, threads).
- An immediate front panel, without writing a UI.
- Visual programs readable by non-developers (technicians, quality managers).
- Standalone executables that are simple to deploy on a bench.
- A Python call from LabCraft (D-14), so as not to pit the two against each other.

## What does not exist yet

To our knowledge, none of these tools combines all of the following: a graphical dataflow language
for measurement and test, an open text format that is the program, an AI agent able to write, run
and fix a program alone, cross-platform execution, a free licence. This is the place LabCraft
aims for.
