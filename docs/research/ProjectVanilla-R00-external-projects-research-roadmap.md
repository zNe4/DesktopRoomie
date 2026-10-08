# Project Vanilla / DesktopRoomie — R00: External-project research roadmap

**Status:** Research plan, not an implementation commitment  
**Version:** 1.0 · 2026-10-06  
**Suggested repository location:** `docs/research/R00-external-projects-research-roadmap.md`  
**Companion references:** `docs/DesktopRoomie-roadmap-v0.1.md`, `docs/DesktopRoomie-A00-A01-plan-v0.1.md`

> **Purpose.** Build DesktopRoomie from the ground up, but learn from existing implementations before we commit to architectural choices. Research should be **just in time**, tied to the next real mission and its acceptance test. We are gathering evidence, not choosing somebody else's codebase as the foundation.

---

## 1. What we learned from the existing-project scan

### 1.1 Novelty, accurately stated

The general categories **AI desktop companion**, **virtual pet with memory and mood**, **Shimeji-style character inhabiting desktop windows**, and **persistent agent with simulated needs/reflexes** already have substantial prior art. It would be misleading to claim that any of those concepts alone were invented here.

What looks distinctive about the *combined Project Vanilla vision* is its intended integration:

- A persistent **character identity** whose mental state is not tied to one UI, desktop, or eventual phone body.
- A genuinely interactive **desktop inhabitant** with user-defined floor, grabbing, falling, walking, foreground/background placement, and eventually ordinary windows as physical surfaces.
- **Authoritative simulation** separated from a representational body and replaceable AI services: the language model does not unilaterally modify facts or durable state.
- Distinct **world truth, observations, beliefs, and memories**: perception may be stale, missing, mistaken, or later corrected.
- A **fast, local reflex / Spine path** that can move the body without waking or invoking the conscious brain; sleep, waking, and interruption policies are explicit.
- Persistent **needs, habits, emotions, relationship history, and autonomy**, including quietness, refusal, and absence handling.
- A **character-specific private content pipeline** for appearance, animation, dialogue, audio, and voice, without forcing that content into the generic engine.
- A **Linux/X11-first desktop implementation** with measured interaction against an actual window manager and compositor, while preserving future Android capability.

**Caution:** “I did not find a single repo with this complete combination” is a result of an exploratory scan, **not a proof of global uniqueness**. Features advertised in READMEs may also differ from implemented behavior. The research missions below exist precisely to inspect the code and its real limitations.

### 1.2 Main conclusions

1. **Mochi is our closest near-peer for body + world-state + LLM + fast physical reactions.** It should be studied before formalizing the Spine / perception interfaces, and its action integration is useful earlier.
2. **AIRI is a strong example of a broad companion platform.** It matters for adapters, voice/LLM integration, runtime ownership, and eventually more than one presentation body; it is not a reason to build a massive platform immediately.
3. **NekoAI and shimeji-rs are practical desktop reference implementations.** These are valuable now, while desktop input, windowing, stacking, motion, and sprite contracts remain active design questions.
4. **Desktop Virtual Buddy is worth studying for its Shimeji behaviors and window-platform physics.** Desktop mechanics should be separated from character reasoning.
5. **Companion Emergence and OpenCrayFish are useful for persistent internal life.** Mood, rhythm, sleep, drives, callbacks, and modular interfaces should be judged by observed behavior, not the biological metaphor alone.
6. **Mana, Open-LLM-VTuber, and NeuralCompanion provide lessons about multimodal assistants and voice pipelines.** Most of their lessons become relevant at A03, A06, and A07, not during early window-host work.
7. **Generative Agents is conceptual background for memory and reflection.** Its architecture is inspiration, not proof that every desktop pet needs the same memory pipeline.

---

## 2. Research ground rules

**Build independently.** We may learn from data structures, tests, design tradeoffs, timing models, debugging interfaces, and mistakes. We should not copy code, asset packs, prompts, character content, or an external architecture wholesale. Any actual reuse requires a separate scope decision and license/attribution review.

**Code over marketing.** A project's README is the starting hypothesis. A completed study identifies the real implementation path: input event → owner/queue → state mutation → action command → renderer or provider → persistence and recovery. Cite precise source paths and, where appropriate, pinned commits.

**Just-in-time depth.** Do a short orientation now; postpone deep dives until the associated adventure is being planned. A research mission should not block an unrelated implementation mission.

**No speculative scaffolding.** DesktopRoomie's current roadmap explicitly treats “Brain,” “Spine,” “sensor,” and “body” first as **logical boundaries**, not a mandate for microservices, a plugin marketplace, or one process per organ. Prove a boundary in one executable before splitting it.

**Keep the simulation authoritative.** Bodies display state and report inputs. External models propose actions/interpretations. Application rules validate state changes. This principle takes priority over external designs.

**Respect local privacy and content provenance.** External research may use public code and synthetic fixtures. Keep privately extracted NEKOPARA game material, character assets, corpus audio, trained models, and real sensor data outside public documentation and repositories.

**Compare alternatives.** For every lesson, write at least one reason *not* to adopt the source design: coupling, cost, portability, latency, permissions, complexity, or maintenance burden.

**Do not advance missions automatically.** Research ends in notes and optional design recommendations; implementing them is a separately approved mission.

---

## 3. Research timing aligned with the real DesktopRoomie adventures

The labels below refer to the existing [canonical roadmap](https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-roadmap-v0.1.md) and [A00/A01 execution plan](https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-A00-A01-plan-v0.1.md). The `R00` identifiers designate **research missions**, not replacements for the implementation numbering.

| When in our roadmap | Main external references | Question we must answer *then* | Depth | R00 mission |
|---|---|---|---|---|
| **Now / before finishing A00 host decisions** | NekoAI, shimeji-rs | Are there robust Linux/X11 patterns for non-activating transparent windows, click-through, compositor quirks, geometry, and workspace control? | Targeted code reading | R00-M01 |
| **A01-G1 layer/workspace/bounds; A01-W follow-up** | NekoAI, shimeji-rs, ShimeLinux | How are stacking, active windows, focus safety, per-window geometry, and window lifecycle represented? | Targeted deep dive | R00-M01/M03 |
| **A01-W1.1 (after stable host and W1)** | Openbox/EWMH/X11 references, shimeji-rs as relevant | Can Vanilla remain between two selected managed application windows in z-order without focus theft, unbounded restacking, or interference with normal window actions? How do raise/focus, transient groups, workspace changes, and target disappearance affect it? | Targeted Openbox/source + live-host experiment, not A00 scope | R00-M03 revisit / A01-W1.1 feasibility |
| **A01-G2/G3 body animation, drag/drop, physics, idles** | shimeji-rs, Desktop Virtual Buddy; Mochi's action layer | How do animation anchors, pointer capture, movement cancellation, platforms, and interruptions stay consistent? | Targeted deep dive | R00-M02/M03 |
| **A02-G1–G3 needs/clock/activity/persistence** | Companion Emergence, OpenCrayFish, Mochi | Which state variables and update/cooldown rules produce legible life without punitive idle needs or constant LLM calls? | Deep dive before A02 design | R00-M04 |
| **A03-G1–G3 offline dialogue, voice, performance** | Open-LLM-VTuber, Mana, AIRI | How do audio queues, synthesis adapters, emotions, playback, interruption, and offline fallback interact? | Targeted | R00-M07 |
| **A04-G1–G3 perception, Spine, sleep, wake** | Mochi, AIRI; Companion Emergence, OpenCrayFish | How do events bypass deliberate processing, wake thresholds work, and stale/error-prone observations become conscious beliefs? | Highest-priority architectural deep dive | R00-M05 |
| **A05-G1–G3 memories, emotional continuity, habits** | Companion Emergence, OpenCrayFish, Generative Agents | How are episodes recorded/retrieved/consolidated, contradictions corrected, and memory growth bounded? | Deep dive | R00-M06 |
| **A06-G1–G3 live conversation / semantic proposals** | AIRI, Mana, Open-LLM-VTuber, NeuralCompanion | How are providers isolated, context filtered, generated actions validated, speech cancelled, and failures handled? | Deep dive | R00-M07/M08 |
| **A07-G1–G3 desktop eyes/ears/weather/activity** | Mochi, NeuralCompanion, AIRI | How are raw screen/sound data gated by permission, abstracted into observations, and kept from leaking to model providers? | Deep dive | R00-M08 |
| **A08/A09 Android home and single identity across devices** | AIRI plus any cross-device agent repos found at that time | What actually owns state; how are mobile lifecycle, reconnect, versioning, duplicate commands, and conflicting hosts handled? | Fresh research; architecture review | R00-M09 |
| **A10/A11 phone life, notifications, dreams, social events** | Companion Emergence, AIRI, OpenCrayFish | What makes background activity feel earned rather than spammy or fabricated? | Targeted | R00-M10 |
| **A12/A13 multi-character/public reusable release** | AIRI, Shimeji pack ecosystems, multi-agent research | How do character packs share an engine safely without shared-memory confusion, license trouble, or runaway complexity? | Later | R00-M11 |

**Important timing:** We need not complete R00 before continuing A00/A01. `R00-M01` is the one study with immediate value; `R00-M02/M03` should land shortly before their first relevant body/window mission. The deep cognitive studies are deliberately deferred until A02–A05.

---

## 4. Repository-specific study briefs

### R00-M01 — NekoAI: practical Linux desktop-host engineering

**Repository:** [nucket/NekoAI](https://github.com/nucket/NekoAI)  
**Best time:** before finalizing remaining A00 decisions; revisit at **A01-G1**.  
**Why study it:** A Rust/Tauri-based AI desktop pet, with Linux/X11 mentioned in its public project materials. It provides a useful counterexample to our current rendering/host choices.

**Research questions**
- What creates and owns the transparent pet surface, and how are non-activating input and click-through regions implemented?
- What is platform-specific, what lives in Rust, and what is delegated to Tauri/webview/platform APIs?
- How are pointer movement, window/desktop bounds, scaling, display changes, and window lifecycle handled?
- Can it actually change stacking/workspace behavior or is it always-on-top? What does it do under fullscreen and focus-sensitive apps?
- What background loops run while idle? How costly is the renderer compared with the content/AI services?
- Are state ownership and host/UI responsibilities cleanly separated or tightly coupled?

**Inspect:** executable/entrypoint, window creation, platform adapters, event loop, input hit regions, window geometry helpers, failure paths, tests and known issues.  
**Output:** a small **A00/A01 host-pattern comparison** against our proven X11/bspwm/Picom behavior.  
**Do not automatically adopt:** Tauri or the project's GUI stack just because it uses Rust. The host technology remains an evidence-driven decision.

**A00 targeted pass completed 2026-10-08:** the source-level Linux-host refresh and the remaining A00-G3 standards research are recorded in [studies/R00-M01-nekoai-linux-host-a00-g3.md](studies/R00-M01-nekoai-linux-host-a00-g3.md). The current disposition is to retain the native Rust + x11rb managed X11 host for A00/A01 unless real-host A00-G3 evidence contradicts it. Revisit NekoAI at A01-G1 only for newly relevant host/recovery behavior; do not redo the completed A00 comparison by default.

### R00-M02 — shimeji-rs: clean Rust/X11 movement and sprite mechanics

**Repository:** [danhab99/shimeji-rs](https://github.com/danhab99/shimeji-rs)  
**Optional reference:** [BujjuIsABee/shimelinux](https://github.com/BujjuIsABee/shimelinux) (Shimeji-style behavioral compatibility).  
**Best time:** **A01-G2/G3** and **A01-W** window interaction extension.  
**Why study it:** Its README identifies it as a Rust-native X11 mascot engine with real Shimeji-ee pack compatibility, walking, climbing, grabbing/throwing, and window interaction. It also documents a cleanroom rewrite methodology—valuable for *our* independent build philosophy.

**Research questions**
- What are the core structs/interfaces for position, velocity, collider/bounds, action, pose, timeline, sprite anchor and direction?
- Is there one authoritative action state machine? How are commands interrupted when grabbed, dropped, or the floor/platform disappears?
- How are animation frames aligned during movement, reverse direction, and transitions?
- How are platform rectangles discovered? How frequently are windows queried and what happens when they move/minimize/disappear?
- How are multi-monitor coordinates, negative origins, panel/work-area constraints, and mouse release outside the pet handled?
- What tests reproduce movement and window geometry without requiring a full live desktop?
- How did its author validate behavioral compatibility *without copying implementation*? What limitations resulted?

**Inspect:** crates/modules, X11 adapter, movement and physics functions, action/behavior XML parsing, pack loader, tests, `docs/CLEANROOM_REWRITE.md` and known-limitations section.  
**Output:** an **A01 movement/animation contract** and a separate **A01-W window-platform risk checklist**.  
**Do not automatically adopt:** the full Shimeji-ee XML behavior format; Vanilla's semantic actions and private sprite pack can be simpler.

### R00-M03 — Desktop Virtual Buddy: physical body and windows as surfaces

**Repository:** [spyderweb47/Desktop-Virtual-buddy](https://github.com/spyderweb47/Desktop-Virtual-buddy)  
**Best time:** **A01-G3.1–G3.7**; revisit at **A01-W**.  
**Why study it:** Its public design emphasizes a Shimeji-style pose vocabulary, throw/drop physics, wall/ceiling travel, application-window platforms, and optional LLM behavior selection.

**Research questions**
- How are the 46-pose/action semantics separated from the renderer and from physics?
- How does pointer intent distinguish click, pet, grab, drag, throw, and release?
- How do window rectangles become walkable/perchable collision surfaces, and what is the stale-geometry recovery behavior?
- Can a high-level planner request actions without directly setting screen coordinates every frame?
- Are multiple pets truly separate agents or only separate sprites sharing one global world?
- What Windows-specific assumptions must not slip into our Linux architecture?

**Inspect:** input controller, physics update, sprite/pose contract, window-platform provider, AI action adapter, state serialization, tests.  
**Output:** a concise **body/action/physics boundary proposal**, with an explicit “we will not copy this” list.  
**Do not automatically adopt:** an entire Shimeji behavior tree or polling approach if our bounded early interactions do not require it.

### R00-M04 — Companion Emergence + OpenCrayFish: internal life, rhythms, needs

**Repositories:** [hanamorix/companion-emergence](https://github.com/hanamorix/companion-emergence), [easonlai/opencrayfish](https://github.com/easonlai/opencrayfish)  
**Best time:** **A02-G1–G3**; revisit for **A04 sleep/reflex** and **A11 background life**.  
**Why study them:** Both present biologically inspired persistent-companion designs: emotional/body rhythms, ongoing processes, local state, and behaviors extending beyond active chat. OpenCrayFish is additionally interesting for manifest-based extensibility and resource-constrained operation.

**Research questions**
- What state is genuinely persisted versus recomputed on boot? How are monotonic time, wall-clock jumps, suspend, and long absence handled?
- Which needs/drives create observable choices? Do many variables cause fragile tuning or contradictory behavior?
- How do mood state and durable relationship/history differ? Do emotional values drift, saturate, recover, or become stuck?
- What decides that the system should stay silent, rest, initiate, or defer an expensive LLM call?
- Is “sleep” a real constraint on perception/action, or merely an animation / timer / prompt instruction?
- What does a background heartbeat own? Can a loop run safely when sensors/providers are missing?
- What is modular in code, not just in metaphor? Are plugin manifests necessary or too expensive for our first build?

**Inspect:** simulation loop/heartbeat, durable state schema, emotion/need update functions, transition rules, persistence and migrations, provider boundaries, loop tests.  
**Output:** **A02 state-transition candidates** and an **absence/catch-up policy comparison**; defer any elaborate organ simulation until validated by gameplay.  
**Do not automatically adopt:** any psychological or “biological” metaphor that lacks clear state ownership and testable outcomes.

### R00-M05 — Mochi, AIRI and persistent-life systems: perception → Spine → brain

**Primary:** [NatBrian/mochi-llm-pet](https://github.com/NatBrian/mochi-llm-pet)  
**Compare with:** [moeru-ai/airi](https://github.com/moeru-ai/airi), Companion Emergence, OpenCrayFish.  
**Best time:** **A04-G1–G3**, with a *light* look at Mochi earlier when planning A01 idle/action scheduling.  
**Why study it:** Mochi advertises screen/cursor/window awareness, a world-state representation, persistent mood/energy/bond, rule-based fallback, and immediate physical interactions alongside LLM-driven behavior. Our fast local reflex idea overlaps strongly and deserves a code-level comparison.

**Research questions**
- What exactly enters through the input/event boundary? Is it a normalized event, global mutable state, screenshot, poll result, or direct callback?
- Which actions run without LLM deliberation and which require it? Is any “reflex” genuinely outside the conscious decision loop?
- What wins if a grab, fall, scheduled walk, voice action, and delayed LLM reply collide?
- How are cancellation, priorities, reentrancy, and stale decisions handled? Can an old model answer move a body whose situation has changed?
- When does the brain wake, and what is the cost of observing the world continuously?
- Could the brain remain **asleep while the body reacts**? If not, which boundary would need to change in our design?
- How should fallible sensors express source/time/confidence and preserve a distinction between **truth → observation → belief → memory**?

**Inspect:** event dispatch, brain invocation, reflex handlers, world-state updater, action scheduler, timeout/cancellation paths, sleep logic, integration tests.  
**Output:** one explicit **Spine event-contract ADR** with example traces for (1) lift while asleep, (2) simulated thunder, (3) stale clock belief corrected on waking, and (4) two conflicting simultaneous actions.  
**Do not automatically adopt:** LLM-driven frame-to-frame motor control, direct model access to raw sensor data, or state mutation buried inside rendering callbacks.

### R00-M06 — Memory architecture: Companion Emergence, OpenCrayFish, Generative Agents

**Repositories:** Companion Emergence, OpenCrayFish.  
**Conceptual source:** [Generative Agents: Interactive Simulacra of Human Behavior](https://doi.org/10.1145/3586183.3606763) (2023).  
**Best time:** **A05-G1–G3**.  
**Why study them:** Our proposed instant/short/mid/long retention horizons and episodic/semantic/associative functions need a small, sustainable, falsifiable implementation—not merely a vector database.

**Research questions**
- What is stored as an event, remembered belief, preference, association, summary, or learned routine? Are facts with different confidence/freshness types mixed together?
- How are importance and retrieval relevance estimated? When is summarization triggered, and can summaries be corrected?
- How are contradictory episodes and mistaken perceptions represented without retroactively rewriting history?
- What gets forgotten and why? Can the database be inspected and bounded after accelerated-month simulations?
- Are memories actually used by *behavior*, or only copied into chat context?
- What is the latency/privacy cost of embedding every event versus simpler tags and timestamps?

**Inspect:** schema, memory capture rules, ranking/retrieval, consolidation jobs, summaries, tests for correction/forgetting, fake-time simulations.  
**Output:** a **memory-record contract + three acceptance scenarios**, including one false belief and one preference learned across sessions.  
**Do not automatically adopt:** one universal “long-term memory” vector store or mandatory reflection loop before simple episodic recall works.

### R00-M07 — Voice and dialogue plumbing: Open-LLM-VTuber, Mana, AIRI

**Repositories:** [Open-LLM-VTuber/Open-LLM-VTuber](https://github.com/Open-LLM-VTuber/Open-LLM-VTuber), [Yuuzulight/Mana](https://github.com/Yuuzulight/Mana), AIRI.  
**Best time:** **A03-G1–G3** (local curated reactions); revisit deeply at **A06-G1–G3** (live generated speech).  
**Why study them:** They offer practical examples of coupling speech recognition / dialogue / synthesis / avatars and a range of local-versus-remote provider choices.

**Research questions**
- How are speech intents, expressions, animations, and subtitles coordinated with real playback completion rather than estimated durations?
- How are TTS jobs cancelled after a newer event? How is a delayed chunk prevented from speaking obsolete emotional intent?
- Can curated voice lines and generated voice use the **same playback interface**?
- Which adapter boundaries allow GPT-SoVITS or another backend to be replaced without affecting character truth or behavior?
- How is audio latency measured, and what remains operational without internet or model availability?
- What is the separation between a provider-generated utterance and an authoritative semantic state change?

**Inspect:** provider interfaces, queues, interruption/streaming logic, transcript-to-action parsing, visual reaction synchronization, offline fallback.  
**Output:** an **A03/A06 audio-performance contract** and a failure/cancellation test checklist.  
**Do not automatically adopt:** a VTuber-first application layout or a network service for every audio stage.

### R00-M08 — Perception, multimodality and grounded conversation

**Repositories:** Mochi, [Rakile/NeuralCompanion](https://github.com/Rakile/NeuralCompanion), AIRI, Mana.  
**Best time:** **A06** for conversational grounding and **A07** for real desktop eyes/ears and external signals.  
**Why study them:** They demonstrate different integrations for screenshots, microphones, webcam, activity context, voice, and conversational memory.

**Research questions**
- How often is the screen captured? Is a cropped/structured observation sufficient, and are windows/app names sampled separately?
- What can be processed locally and what may be sent to a remote provider? Is user consent per sensor and per destination explicit?
- How are stale or absent sensor reports tagged? What would it take to intentionally simulate an incorrect estimate and later update it?
- How is the LLM context built **from allowed observations/beliefs** rather than omniscient state or raw unfiltered screenshots?
- How does the system handle user interruption, microphone access, privacy indicators, and deletion/revocation?
- Does provider failure stop perception, or can the simulation continue with limited local observables?

**Inspect:** capture adapters, permissions and redaction, sensor sampling, model context assembler, logs/retention defaults, provider-call path, tests.  
**Output:** an **A07 sensor-permissions/threat-model checklist** and a *minimal* observation type with source, timestamp, confidence and scope.  
**Do not automatically adopt:** continuous full-screen upload, always-on microphones, or unconditional permanent raw sensor logs.

### R00-M09 — AIRI and cross-device identity

**Primary:** AIRI. **Discover additional live implementations when we actually reach A08/A09.**  
**Best time:** **A08-G1–G3 and A09-G1–G3**.  
**Why study it:** AIRI spans more than a single avatar style or desktop surface and may offer useful examples of platform/boundary packaging. But being multi-surface does not necessarily mean it solves authoritative distributed simulation.

**Research questions**
- Where does the authoritative identity, simulation clock, and durable memory actually live?
- Are desktop/mobile clients renderers or independent active agents? How are schemas/version negotiation handled?
- What happens when the owner is offline, when the phone resumes after hours, or when requests arrive twice/out of order?
- Does a reconnect replay events or reconcile full snapshots? Are conflicting edits possible?
- What does mobile background execution permit in reality? What battery and lifecycle measurements support the architecture?

**Inspect:** state ownership, transport/API contracts, platform lifecycle, sync/retry logic, migrations, reconnection tests.  
**Output:** an **A08/A09 authority and reconnect ADR**, including explicit rules forbidding two independent Vanillas.  
**Do not automatically adopt:** distributed state ownership or always-online cloud dependency merely to support two UIs.

### R00-M10 — Dreams, autonomous life, notifications

**Repositories:** Companion Emergence, OpenCrayFish, AIRI.  
**Best time:** **A10/A11** after persistent state, memory, and conversation are already reliable.  
**Research questions:** What is the causal source of dreams/background activity? Can the user understand why an event happened? How do timers and quiet hours avoid spam? Are notifications opt-in and reversible? Does fictional social content contaminate ordinary world facts or real user memories?  
**Output:** bounded background-activity and notification policies with clear source/provenance markers.  
**Do not automatically adopt:** simulated social networks or background LLM chatter as a substitute for meaningful autonomous behavior.

### R00-M11 — Multiple characters and portable public engine

**Primary:** AIRI and relevant Shimeji-compatible pack systems. Add contemporary multi-agent studies at **A12/A13**, not now.  
**Research questions:** How are individual private states separated from shared-world state? How do two characters avoid contradictory claims about shared events? Which contracts belong to reusable DesktopRoomie rather than private Vanilla? How are pack schemas, licenses, migration, permissions and safe public synthetic fixtures designed?  
**Output:** a tested **two-character ownership model** and a **public/private content boundary** for any released engine.

### R00-M12 — Spine 2D runtime and private asset feasibility

**Input:** a private official chibi Vanilla Spine 2D export is available as `.skel`, `.atlas`, and `.png`, with multiple existing animations. The copyrighted asset files remain private and are not to be committed to DesktopRoomie.  
**Best time:** immediately after A00-G3 and before **A01-G2.1**, alongside the targeted A01 movement/animation studies rather than during host proof work.  
**Why study it:** this may replace a custom frame-sheet-first pipeline with a skeletal animation path that preserves authored anchors, interpolation, skins/attachments, and a richer existing animation repertoire. The decision must be based on the export's actual Spine version/features, runtime feasibility, licensing, and rendering costs.

**Research questions**
- What exact Spine export/runtime version does the binary `.skel` require, and can that version be identified without redistributing the private asset?
- Which animations, skins, slots, attachments, draw-order changes, events, constraints, and atlas pages are present and actually useful for A01?
- What are the authored origin, scale, bounds, facing conventions, and per-animation extents? Can one stable body/action coordinate contract cover them?
- Which maintained Spine runtime paths are viable with the current Rust/native-X11 host, and what rendering backend or bridge would each require?
- What are the runtime/editor license obligations for a private application and for any future public DesktopRoomie engine? Can public tests use a synthetic/public Spine fixture while the official Vanilla files stay private?
- How do alpha mode, texture filtering, atlas packing, clipping/mesh attachments, interpolation, and animation mixing affect visual correctness and CPU/GPU cost?
- Can semantic actions such as idle/look/walk/sit/sleep/held/fall/land/touch map cleanly onto the available animation set, including interruption and cross-fade rules?
- If direct runtime integration is impractical, is offline rendering to a derived private frame set a legitimate fallback, and what behavior/quality would be lost?

**Inspect:** the private export only in an approved local/private workspace; official Spine runtime/version/licensing documentation; maintained runtime implementations and their rendering interfaces; a synthetic or redistributable fixture for public code/tests.  
**Output:** a **Spine feasibility ADR + private animation inventory + public runtime smoke-test plan**, ending in an explicit choice among direct skeletal runtime, private offline bake, or deferral.  
**Do not automatically adopt:** an unofficial reverse-engineered runtime, redistribution of official Vanilla assets, a renderer/framework rewrite merely to host Spine, or a frame extraction pipeline before the authored skeletal option is evaluated.

---

## 5. Suggested execution order (do not front-load every study)

### Immediate, lightweight work: current A00 → early A01

**Priority 1:** the targeted A00 pass of `R00-M01` is complete; use its recorded A00-G3 findings while finishing host selection, and revisit only if real-host evidence exposes a missing failure mode. **Priority 2:** immediately after A00-G3, run `R00-M12` before A01-G2.1 so the new private Spine 2D export is evaluated before committing to a custom frame-sheet pipeline. **Priority 3:** run `R00-M02` and `R00-M03` when A01 movement/animation/physics implementation is about to start. A *brief* look at Mochi's event-to-action flow may be useful when we introduce A01's replaceable idle scheduler, but the full brain/Spine *cognition* study (`R00-M05`) still waits for A04.

**Checkpoint:** a one-page note that answers “what implementation issue do we now understand better?” and records whether the lesson requires a code change. A note with no recommended change is a perfectly valid result.

### Just before A02

Study `R00-M04` with emphasis on **state update frequency, persistence, elapsed-time semantics, and absence**. Select only a few meaningful needs first. Do not block a working daily-life release on advanced emotions.

### Just before A03/T2 integration

Study the relevant parts of `R00-M07`; use our existing private voice experiments and corpus pipeline as test inputs without importing those files into public fixtures. Offline curated reactions can ship independently of live voice generation.

### Just before A04 and A05

Do `R00-M05` and `R00-M06` as the first major architectural source studies. The actual acceptance scenes—lift while asleep, simulated thunder, mistaken time estimate, correction on wake, remembered preferences—must drive the comparison.

### Just before A06/A07

Return to `R00-M07/M08` for model adapters, context grounding, sensor privacy, cancellation, and failure handling.

### Just before A08 onward

Refresh external research. Public projects evolve quickly; do not assume findings from 2026 still accurately describe them at the time mobile sync or multi-character support becomes active. Perform `R00-M09` through `R00-M11` only when their dependencies are real.

---

## 6. Deliverable format for each research mission

Use one file per study under `docs/research/studies/`, and one continuously updated cross-project ledger. Recommended template:

```markdown
# R00-MXX — [Project / subsystem]

## Decision we are about to make
- Roadmap gate: Axx-Gy / pending implementation mission
- Acceptance scene and measurable constraints

## Source audit
- Repository URL, commit SHA, date reviewed, license
- Relevant source paths and entrypoints
- README claims checked / claims not yet verified

## Reconstructed runtime path
- Event/input -> queue/router -> state owner -> policy -> output
- Persistence and crash/reconnect behavior
- Thread/process boundaries and cancellation behavior

## Strengths
- Observed implementation choices + source links

## Limitations / failure modes
- What seems fragile, costly, platform-specific, or untested

## Alternatives for DesktopRoomie
- Option A: simple native implementation
- Option B: more abstract implementation when requirements grow

## Decision / open questions
- Adopt as principle, reject, defer, or needs a targeted experiment
- Why; when to revisit

## Proposed validation
- A small deterministic test or acceptance-scene reproduction
```

**Minimum evidence to call a study complete:** (1) actual code paths identified, (2) one realistic failure/cancellation path understood, (3) at least one measurable tradeoff, (4) an explicitly scoped recommendation or decision to defer, (5) source commit and license recorded. Do not equate “README read” with “architecture audited.”

A comparison ledger could have these columns:

| Decision area | Reference implementation | Evidence (file/commit) | Approach | Strength | Risk/limitation | Project Vanilla disposition | Revisit trigger |
|---|---|---|---|---|---|---|---|
| Reflex routing | Mochi | *To inspect* | *TBD* | *TBD* | *TBD* | Research at A04 | Before A04-G2 |
| Linux window input | NekoAI | *To inspect* | *TBD* | *TBD* | *TBD* | Research at A00/A01 | Host/input decision |
| Window platforms | shimeji-rs | *To inspect* | *TBD* | *TBD* | *TBD* | Defer until A01-W | Perch implementation |
| Long-term memory | Companion Emergence | *To inspect* | *TBD* | *TBD* | *TBD* | Defer until A05 | Memory architecture |

---

## 7. Specific design risks we should watch for across all projects

| Risk | What to verify in source | Project Vanilla rule of thumb |
|---|---|---|
| LLM controls body too directly | Per-frame decisions; model writes durable state | Model proposes intentions; local rules execute or refuse them |
| “Reflex” still blocks on reasoning | Call graph, queue handoff, wake policy | Fast body response survives model unavailability and sleep |
| State duplicated in UI or adapters | Multiple mutable copies with hidden synchronization | Authoritative owner; renderer gets state views |
| Always-on activity wastes resources | CPU/VRAM/network when idle, poll frequency | Event-driven/low-frequency work unless necessary |
| Physics/actions race each other | Drag vs walk vs fall; cancellation and stale callbacks | One clear action owner, priorities, explicit cancellation |
| Window observations become stale | Minimize, resize, fullscreen, workspace, display change | Freshness and safe fallback; no phantom platforms |
| Sensor data is overcollected | Full-screen upload/retention/provider permissions | Minimize, label, redact, expire and request consent |
| Mood becomes opaque | Explainability and tuning controls | Every important behavior has inspectable causes |
| Memory grows without correction | Summaries, false beliefs, duplicate facts | Provenance, correction and bounded storage |
| Plugin architecture arrives too early | Many adapters/registries before multiple implementations | Logical boundaries first; extract framework when justified |
| Cross-device “sync” creates two personalities | Split-brain ownership, conflicting writes | One identity and one authoritative simulation |
| Code/content licensing gets blurred | Per-file licenses, game assets, sprite packs | Public source and synthetic examples; private character media separate |

---

## 8. Reference directory and source reliability

The following repository identities and their README-level scope were checked on **2026-10-06**. That verification does **not** replace a source-level audit. Links should be paired with an exact commit when the corresponding R00 mission begins.

**Primary studies**

- **Mochi:** https://github.com/NatBrian/mochi-llm-pet — LLM desktop pet, scene observations, body actions, fallback behavior.
- **Project AIRI:** https://github.com/moeru-ai/airi — broader virtual-character ecosystem and platform integration.
- **NekoAI:** https://github.com/nucket/NekoAI — Rust/Tauri desktop-pet host and platform behavior.
- **shimeji-rs:** https://github.com/danhab99/shimeji-rs — Rust/X11 mascot, Shimeji behavior and geometry; also read `docs/CLEANROOM_REWRITE.md`.
- **Desktop Virtual Buddy:** https://github.com/spyderweb47/Desktop-Virtual-buddy — Shimeji-style body, physics, window-platform interaction, optional LLM.
- **Companion Emergence:** https://github.com/hanamorix/companion-emergence — continuous character state, emotional/physical rhythms, local life.
- **OpenCrayFish:** https://github.com/easonlai/opencrayfish — persistent edge companion, plugin boundaries, local operation.

**Supporting / later studies**

- **ShimeLinux:** https://github.com/BujjuIsABee/shimelinux — Shimeji pack behavior and compatibility comparison.
- **Open-LLM-VTuber:** https://github.com/Open-LLM-VTuber/Open-LLM-VTuber — conversational/audio/avatar adapters.
- **Mana:** https://github.com/Yuuzulight/Mana — local-first conversation/voice pipeline and privacy design.
- **NeuralCompanion:** https://github.com/Rakile/NeuralCompanion — multimodal perception, voice, memory/character presentation.
- **Generative Agents research:** https://doi.org/10.1145/3586183.3606763 — memory/retrieval/reflection/planning baseline.

**ProjectVanilla / DesktopRoomie grounding**

- https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-roadmap-v0.1.md
- https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-A00-A01-plan-v0.1.md
- https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-A00-M01-desktop-probe.md
- https://github.com/zNe4/DesktopRoomie/blob/main/docs/DesktopRoomie-A00-M02-dragging-and-release.md

---

## 9. Working agreement for future planning

1. At the start of an implementation mission, check whether its matching `R00` study is due.
2. If yes, study **only the components that inform the immediate decision**, with read-only repository inspection first.
3. Record evidence and alternatives in `docs/research/studies/` and summarize any cross-project principle in the ledger.
4. Draft our own implementation contract and acceptance tests. A source project's design is never binding.
5. Request explicit approval before modifying the DesktopRoomie repository or expanding scope.
6. Re-run the relevant study if a later bug exposes an unexamined failure mode or the external project materially changes.
7. Keep the playable companion advancing; external research is a tool, not a new multi-month dependency.

**First practical research trigger:** complete `R00-M01` for Linux/X11 host behavior as the remaining A00/A01 platform choices arise. Deep work on Mochi's Spine/brain boundary should occur before **A04**, not before the first playable desktop body exists.
