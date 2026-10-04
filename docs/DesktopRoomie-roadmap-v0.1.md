# DesktopRoomie — A roadmap toward Project Vanilla

Version 0.1 · 2026-10-04 · Proposed, living roadmap

Repository: [zNe4/DesktopRoomie](https://github.com/zNe4/DesktopRoomie)

Source: *Project Vanilla — All Ideas / Design Inventory v0.1*, supplied for this planning pass. The repository was inspected on 2026-10-04; it contained the initial `README.md`. The user reports that TTS training is underway. Other implementation status has not been audited here.

This document proposes a route through the entire inventory. It does not turn every possibility into a commitment. Product principles are distinguished from suggested sequencing, experiments, and optional extensions. Dates and effort estimates belong in later adventure plans, after we have measured actual progress.

## 1. The destination

Build a persistent companion whose present behavior makes sense because of what happened before. She can act, remain quiet, sleep, refuse, remember, misunderstand, learn, and continue her life without requiring a conversation or an AI request to keep her running.

**DesktopRoomie** is the working name for the reusable software. **Project Vanilla** is its first private character implementation. These are two views of the same development effort, not two separate products we must build simultaneously.

The first destination is modest: a small companion living on the Arch Linux desktop. The larger destination is one character identity, with multiple bodies and interfaces, living in a persistent world. The extended destination includes her own fictional phone, pictures, social events, dreams, and eventually a shared household with other characters.

The plan grows that experience in playable increments. We should be able to stop after an adventure and still have a worthwhile product.

## 2. Our units of work

| Level | Meaning | Example |
| --- | --- | --- |
| Roadmap | The route through the product vision | This document |
| Adventure | A connected group of goals ending in a runnable product increment | A02: A daily life |
| Goal | A meaningful capability within an adventure | Persistent needs and activities |
| Mission | A bounded implementation assignment prepared when its goal is ready | Save and restore the current activity |
| Steps | Concrete programming instructions within a mission | Files, interfaces, changes, checks |

An adventure is roughly a **release milestone built from vertical slices**: each slice connects enough state, behavior, presentation, and persistence to produce something observable. A goal often corresponds to what software teams call an epic. These terms are organizational aids; “adventure” is a perfectly useful name for our workflow.

The assistant handles planning, architecture, design, implementation briefs, and review. The user implements and runs the software on their machine. We refine the active adventure together using actual behavior, logs, screenshots, measurements, and user experience.

We detail one adventure at a time and keep the next one sketched. We do not generate years of implementation tickets now.

## 3. Principles to preserve as the design changes

1. **One identity and one authority.** Bodies send inputs and show state. They do not independently invent divergent Vanillas. Shared world facts and each character's private mental state have explicit owners.
2. **The simulation owns durable state.** LLMs can interpret, propose, summarize, and speak. Validated application rules decide which persistent changes actually occur.
3. **Truth, observation, belief, and memory are distinct.** A computer knowing something does not imply that Vanilla perceived or remembers it. Confidence, freshness, and provenance become meaningful as cognition develops.
4. **Life works offline.** Needs, sleep, activities, basic emotions, reflexes, local dialogue, routines, and persistence remain available without cloud services. Unsupported online capabilities become temporarily unavailable without stopping the companion.
5. **Silence and independence count as behavior.** Delaying, declining, turning away, sitting nearby, or continuing an activity can be the correct response.
6. **Modularity is about ownership and contracts.** A Brain, Spine, sensor, or service is initially a logical boundary. It does not automatically require a separate executable, network server, plugin framework, or biological simulation.
7. **Private content stays separate.** Extracted game assets, dialogue corpus, curated audio, private models, and sensor records stay outside the public repository. Public examples use suitable original or otherwise publishable assets.
8. **Evidence chooses technology.** Rust and Python have meaningful roles. Additional languages and frameworks must solve a demonstrated problem. Provider names, memory models, behavior algorithms, and host devices remain replaceable choices.
9. **Every adventure delivers an experience.** Infrastructure work earns its place by enabling that adventure's acceptance scene. Debugging and small polish improvements accompany every increment.

## 4. The route at a glance

The numbering is the suggested order for one programmer. The dependency column names essential foundations, not every earlier adventure. Independent work may move earlier when there is a practical reason; the existing playable build remains the integration point.

| ID | Adventure | Product you can run at the end | Essential foundation |
| --- | --- | --- | --- |
| A01 | First visit | A small interactive desktop body | None |
| A02 | A daily life | A persistent offline companion with needs and activities | A01 |
| A03 | A familiar voice | An expressive companion using local speech and reactions | A02; a small usable content pack |
| A04 | A body with a mind | Sleep, limited perception, reflexes, and belief revision | A02 |
| A05 | A shared history | Memories, habits, preferences, and a slowly changing relationship | A04 |
| A06 | A real conversation | Live dialogue grounded in the same character state | A03, A05; usable provider adapters |
| A07 | The world outside the window | Opt-in surroundings influence her perceptions and behavior | A04; memory features use A05 |
| A08 | A second home | An Android room connected to the same identity | A02 state boundary; reuse later features as available |
| A09 | One life across devices | Tested ownership, reconnect, and continuity when the desktop is off | A08 |
| A10 | Her own phone | Character-paced messages, a camera/gallery, and bounded apps | A03, A05; A09 for cross-device delivery |
| A11 | A life beyond our interactions | Fictional social events, dreams, and optional curated information | A05; A10 for phone presentation |
| A12 | A shared household | Two characters living in one consistent world | A05; a second suitable character pack |
| A13 | A home we can keep | A packaged, recoverable, maintainable release of the chosen scope | The adventures selected for that release |

Useful stopping points:

- **After A03:** the first playable target from the inventory: desktop body, needs, persistence, interactions, local voice, and debugging. An experimental TTS command can accompany it without being a runtime dependency.
- **After A06:** a substantial private desktop companion with cognition, history, and live conversation.
- **After A09:** one persistent identity accessible across desktop and phone, with an explicit policy for unavailable hosts.
- **After A11:** the broad private vision, including an inner and fictional social life.
- **A12:** an optional expansion into a household; it does not block releasing the single-character product.
- **A13:** release preparation can be applied at any stopping point. Its broadest version packages the complete chosen vision; it is not permission to postpone usability until the end.

## 5. The adventures

### A01 — First visit

**Experience:** Launch the program and meet a small character who can move, react to the pointer, and be picked up.

**Connected goals**

- **A01-G1: A viable desktop host.** Establish the smallest build/run path on the actual Arch Linux/bspwm setup. Test transparency, stacking, pointer handling, screen boundaries, and clean shutdown before choosing a rendering/window stack for broader use.
- **A01-G2: A readable body.** Render a few animations through semantic actions such as idle, walk, sit, and sleep. Define only the animation metadata currently needed: timing, loop behavior, anchors, direction, hit regions, and interruption/transition rules.
- **A01-G3: A physical interaction loop.** Add hover/gaze, click, grab, hold, drag, and drop/fall behavior. Add a simple pet/poke response and a developer view of the current action. Save minimal settings and position.

**Finish scene:** Launch, walk, pick up, move, drop, pet, quit, and reopen. The body stays usable and respects the current screen geometry. No AI service is required.

**Choose here:** window/rendering approach and the first asset format, based on a small executable comparison if needed. Use placeholder assets if the private chibi pack is not ready. This adventure establishes no advanced cognition or full physics engine.

### A02 — A daily life

**Experience:** She has reasons to do things, and her day survives closing the application.

**Connected goals**

- **A02-G1: Persistent life state.** Introduce a simulation clock, minimal needs, current activity, basic mood, cooldowns, and a short event history. Separate the authoritative state from rendering. Support safe saves and a defined elapsed-time policy after restart or suspend.
- **A02-G2: Activities and items.** Connect needs to eating, drinking, resting, sleeping, play, and attention. Start with a few useful needs; hunger, thirst, tiredness, cleanliness, boredom, and energy remain the candidate set. Items carry properties and tags rather than requiring one behavior implementation per item.
- **A02-G3: Understandable choices.** Choose actions using a simple explainable policy. Utility AI is a candidate, not a prerequisite. Include refusal, interruption, nonverbal behavior, and state-sensitive escalation for repeated poking. Expose why an action was chosen.

**Finish scene:** Offer food, interrupt play, let her become tired, watch her choose rest, then reopen after an absence. State and behavior remain coherent. Decide explicitly how absence affects needs; avoid turning an offline companion into a punishment for not running the application.

**Choose here:** minimal need/mood variables, initial behavior policy, persistence approach, and catch-up rules. Advanced mood dynamics and sleep cognition come later.

### A03 — A familiar voice

**Experience:** Her ordinary reactions sound and look like her, with no inference required during everyday play.

**Connected goals**

- **A03-G1: A local dialogue and reaction pack.** Connect a small reviewed set of lines and nonverbal sounds to meaningful situations. Grow toward the inventory's approximate 50–200-line first-playable range when content is ready; line count is not the acceptance test.
- **A03-G2: Contextual selection.** Use conditions, exclusions, intent, delivery, weight, cooldowns, and semantic families to select appropriate lines without obvious repetition. Keep silence as an available outcome.
- **A03-G3: Coordinated performance.** Connect speech/reaction playback to expression, animation, speaking state, interruptions, and post-speech transitions. Establish a replaceable voice boundary; attach an experimental TTS command when the existing experiment produces a usable backend.

**Finish scene:** A short session includes feeding, petting, refusal, a sleepy reaction, and quiet time. Audio and movement agree. Disconnect the network and repeat the ordinary session successfully.

**Choose here:** playback/queue rules and the initial dialogue record format. Full lip synchronization, a huge content library, and a winning fine-tuned model are not gates. Existing reviewed audio or provisional output can demonstrate the integration.

### A04 — A body with a mind

**Experience:** She reacts physically without having to consciously understand everything, and sleep changes what she knows.

**Connected goals**

- **A04-G1: Perception and belief.** Introduce typed observations with source, time, confidence, and freshness. Keep observable facts separate from beliefs. Start with body movement and time; use controlled test signals before real sensors.
- **A04-G2: A minimal Spine.** Route selected signals to fast body responses and, separately, to conscious processing when appropriate. Implement a small set of sleep/startle/posture reactions and wake thresholds. Balance, protective movement, and other autonomic refinements can grow from this boundary.
- **A04-G3: Actual sleep and waking.** Reduce or suspend selected observations during sleep, allow stale beliefs, and request fresh observations on waking. Distinguish a reflex-only event from one consciously perceived and eligible for memory.

**Finish scene:** Lift her gently while asleep: her body shifts and she remains asleep. A simulated thunder event startles her and may cross the wake threshold. After waking, a reliable clock corrects her rough estimate of how long she slept.

**Choose here:** how much uncertainty and reflex detail improves the experience. Store confidence and source information without inventing elaborate probabilistic machinery for every signal. Generated dialogue can describe belief revision later; the underlying scene already works without an LLM.

### A05 — A shared history

**Experience:** Yesterday and the preceding weeks influence today.

**Connected goals**

- **A05-G1: Memory with consequences.** Add immediate context, notable episodes, learned facts/preferences, and emotional associations. Give records provenance, importance, retrieval rules, and a lifecycle for summarization, consolidation, forgetting, and correction. Some associations may influence behavior without explicit recall.
- **A05-G2: Relationship and emotional continuity.** Separate slow familiarity, trust, attachment, comfort, and conflict residue from short-term mood. Introduce only useful mood dimensions; valence, irritation, affection, anxiety, playfulness, sociability, concern, and curiosity remain candidates. Tune inertia, recovery, event impulses, bounded variation, and hysteresis where observable behavior needs them.
- **A05-G3: Habits and discovery.** Learn simple temporal patterns statistically, recognize unusual absences/session lengths, and learn item preferences. Add a small set of rare events with long cooldowns. Keep exact triggers in developer tooling, not ordinary UI.

**Finish scene:** Across several sessions, she learns a preference, recalls a meaningful event, notices an unusual routine, and remains affected by history after restart. A brief annoyance does not erase established trust. Use accelerated developer time to inspect long-term behavior, then validate it in ordinary use.

**Choose here:** reconcile memory taxonomies. Instant/short/mid/long-term are retention horizons; episodic/semantic/associative are functions and may coexist as separate dimensions. Decide actual storage/retrieval mechanisms only after a small case works. Do not make every interaction a permanent memory.

### A06 — A real conversation

**Experience:** You can talk freely, and the speaking character is the same one who has been living on the desktop.

**Connected goals**

- **A06-G1: Grounded language.** Build context from character guidance, allowed observations, relevant beliefs/memories, activity, and relationship. Generate Japanese speech text through a replaceable LLM provider. The model does not receive omniscient world state by default.
- **A06-G2: Interpretation without surrendering authority.** Translate language into proposed semantic events such as apology, rejection, playful intent, or requests. Validate consequences and capabilities in the application. Generated claims and uncertain interpretations do not silently become world facts or durable memories.
- **A06-G3: Responsive conversation.** Integrate reference-aware TTS, cancellation, delayed/refused responses, provider failures, and fallback behavior. Add sentence streaming if measured latency justifies it. Allow opt-in retention and later review of generated lines for the offline library.

**Finish scene:** Discuss a previous interaction, apologize while she is annoyed, and ask for an activity she may decline. Inspect the resulting state changes. Interrupt speech and disconnect a provider; the companion keeps living and does not apply obsolete outputs to a new situation.

**Choose here:** LLM adapter, context budget, semantic interpretation rules, and live voice backend. Mood informs intended delivery but does not map mechanically to one TTS label. A working text/reaction fallback remains available if live synthesis is too slow.

### A07 — The world outside the window

**Experience:** Selected surroundings influence her behavior through believable, bounded perceptions.

**Connected goals**

- **A07-G1: A local sensor boundary.** Add explicit permission, local processing, retention, and external-sharing rules for each adapter. Start with inexpensive signals such as device availability, charging, presence, time, or coarse activity.
- **A07-G2: Meaningful surroundings.** Add selected weather and music/context signals. Desktop eyes or sound sensing are later adapters within this goal, introduced only if their benefit justifies access. Convert raw input into limited observations before it reaches cognition or external models.
- **A07-G3: Familiar places.** Develop the place/familiarity model using simulated or manually supplied data first. Add GPS, Wi-Fi, geofence, or motion adapters when Android supports them. Keep anonymous place IDs local, learn familiarity, and accept user-taught labels such as home or a friend's house.

**Finish scene:** A weather or presence event changes behavior; stale input remains uncertain. An unfamiliar place becomes familiar after repeated visits. Revoking a sensor permission stops new observations, and the debug view shows what information could be sent externally.

**Choose here:** useful sensors, retention defaults, place learning/decay, and observation granularity. This adventure can ship with a small sensor subset; Android-specific sensing joins A08/A09. Raw coordinates or desktop contents are not automatically exported with a fictional abstraction.

### A08 — A second home

**Experience:** Open an Android room and find the same character, with the same needs and recent history.

**Connected goals**

- **A08-G1: A body-independent interface.** Expose state views, authorized commands, events, and capability availability without coupling the runtime to desktop rendering. Start with a single authoritative host and a clear connected-client model.
- **A08-G2: An Android room.** Provide a small room with useful activity locations: bed, food, toys, and an abstract bath as those activities exist. Translate touch interactions into the same meaningful commands used by the desktop.
- **A08-G3: Honest disconnection.** Add authenticated pairing, connection visibility, compatible state/schema versions, and bounded offline input handling. A disconnected body shows a stale/unavailable view or explicitly pending actions; it does not silently start another authoritative simulation.

**Finish scene:** Feed her on the phone and observe the updated state on desktop. Reconnect after a dropped connection without duplicating the meal. Restart the phone UI without creating a new identity.

**Choose here:** Android UI language/framework and the first transport. The desktop is a reasonable development authority initially, not a permanent architectural commitment. This adventure may require the host to remain on; A09 addresses that limit explicitly.

### A09 — One life across devices

**Experience:** Her continuity has a tested home even when a body disappears or the desktop is turned off.

**Connected goals**

- **A09-G1: Prove an authority host.** Measure idle CPU, memory, battery, lifecycle behavior, storage reliability, and background execution on the actual candidate phone. Evaluate whether it should own the core. Another explicitly chosen host remains possible; cloud inference workers never acquire state authority merely by serving requests.
- **A09-G2: Reliable commands and recovery.** Handle duplicate/reordered delivery, conflicting or stale commands, reconnect, snapshots, migrations, and crashes. Define what is queued, rejected, or revalidated. Test any ownership transfer so two hosts cannot both accept authoritative writes.
- **A09-G3: Continuity with realistic time.** Advance or reconstruct elapsed simulation safely after suspension. Separate intentional character delays from transport delays. Decide which scheduled actions need a running host and which can be reconciled on return.

**Finish scene:** With the selected authority running, turn the desktop off, use the phone, then reconnect the desktop. State does not rewind or duplicate effects. Suspend or stop the authority itself and verify the documented recovery policy.

**Choose here:** authoritative device, supported offline behavior, migration strategy, and compatibility policy. We cannot promise real-time actions while every host is stopped. If phone-hosting tests fail, keep A08 usable and revise the deployment goal; do not simulate two independent lives and merge them later.

### A10 — Her own phone

**Experience:** She can message at appropriate moments, take a fictional picture, and revisit her gallery.

**Connected goals**

- **A10-G1: Character-paced communication.** Add messages, voice messages, occasional pictures, delayed replies, and memory-aware invitations for food or interaction. Enforce quiet hours, rate limits, dismissal/mute controls, and user-selected boundaries. Notifications serve behavior rather than engagement targets.
- **A10-G2: Bounded fictional apps.** Introduce Messages, Camera, Gallery, and simple Notes. Weather reuses world events. Reserve News, Nekogram, and Games for the content that actually needs them. Every app action is an explicit, inspectable capability; an LLM can propose an action but cannot grant itself access.
- **A10-G3: Pictures with continuity.** Pass semantic intent to a replaceable image service. Construct output from canonical appearance, outfit, room, activity, mood, pose, and framing. Store selected images and metadata in a persistent gallery, linked to memory where appropriate.

**Finish scene:** She sends a contextually appropriate message through an allowed channel, can wait before replying, and respects mute settings. She creates or selects a picture and later remembers the associated event. If generation is unavailable, existing gallery and messaging features still work.

**Choose here:** in-app versus external delivery, image provider, gallery retention, and capability boundaries. “Camera” initially means the fictional picture workflow, not access to the user's physical camera. External messaging integrations require their own explicit scope.

### A11 — A life beyond our interactions

**Experience:** Her world produces small causes for behavior that the user did not initiate.

**Connected goals**

- **A11-G1: A fictional social environment.** Add a bounded Nekogram model: fictional accounts, posts, reactions, persistent threads, and small events. These can initially be authored or rule-driven; full simulations of every account are unnecessary.
- **A11-G2: Sleep with narrative consequences.** Occasionally derive dream records from memories, mood, fictional events, and abstract material. Preserve dream provenance so a remembered dream does not become an actual external event.
- **A11-G3: Selective outside information and activities.** Add opt-in structured inputs for holidays, weather, selected local information, or curated topics/news. Small fictional phone games and notes can create activity history when useful. Keep source identity, freshness, and capability limits visible to the system.

**Finish scene:** A fictional social event affects her mood, a dream becomes a later conversational hook, and a selected outside event is used appropriately. Run offline and confirm that local world activity still exists. No arbitrary web browsing is required.

**Choose here:** narrative frequency, dream effects, source selection, and content production method. Tune for occasional meaningful events; more autonomous activity is not automatically a better experience.

### A12 — A shared household

**Experience:** A second character shares the room without inventing a contradictory world or sharing Vanilla's private mind.

**Connected goals**

- **A12-G1: Shared facts, distinct characters.** Give both characters the same authoritative objects, locations, and events while preserving separate observations, beliefs, memories, needs, and relationships.
- **A12-G2: Interactions and resources.** Handle turn-taking, object ownership/use, attention, conversation participants, and character-to-character events. Extend relationship records without assuming every relationship is with the user.
- **A12-G3: Prove a second pack.** Load a second suitable character pack and run common activities without embedding character-specific exceptions in the engine. Use the preserved multi-character corpus context for private design research where applicable.

**Finish scene:** Two characters want the same item; one uses it, both observe appropriate consequences, and only the character who witnessed a private event initially knows it. Save/reload preserves their distinct histories.

**Choose here:** scheduling/fairness, multi-character performance budget, and actual pack extension needs. This is an optional expansion. The public engine can first ship for one character if that is the useful scope.

### A13 — A home we can keep

**Experience:** Install the chosen product scope, live with it, update it, and recover it without development rituals.

**Connected goals**

- **A13-G1: Everyday quality.** Tune repetition, silence, natural refusals, activity transitions, interruptions, utility curves, emotional recovery, memory relevance, speech length, expressive references, delays, and rare-event frequency. Add small physical interactions that reward observation. Hide ordinary meters while retaining accessible diagnostics.
- **A13-G2: Delivery and recovery.** Package supported hosts, configuration, provider options, private pack installation, state backup/restore, migrations, and diagnostic export. Validate upgrades on copied saves and define recovery from a failed upgrade. Measure resource use on the user's real machines.
- **A13-G3: A reusable engine boundary.** Document the supported pack and service contracts. Demonstrate an installable public sample without private copyrighted content. Generalize only the proven seams; public release scope and licensing are separate decisions when distribution is planned.

**Finish scene:** Install in a clean environment, attach the intended pack, run an ordinary session offline, back up the identity, upgrade, and recover from a deliberately failed copied-save migration. Then use the release over ordinary days to identify pacing and reliability problems that a demo cannot reveal.

**Choose here:** supported platforms, release scope, resource budgets, update mechanism, and public/private packaging. A13 applies to whichever stopping point we select; optional later adventures need not block a stable release.

## 6. Supporting tracks

These feed the adventures. They are not a mandatory infrastructure campaign before A01. Existing work should be audited and reused, not rebuilt merely to fit the new repository.

### T1 — Corpus, provenance, and curation

**Feeds:** A03 content, A05 characterization, A06 context, A12 additional characters.

Maintain immutable extracted originals and explicit derived copies. Import through engine-specific adapters, including the inventory's CatSystem2 and KiriKiri/FreeMote-derived paths, into one canonical schema. Recover script-to-voice links from source data rather than using ASR as the primary mapper. Preserve full dialogue context before filtering to Vanilla; retain the other main catgirls' data for future research and packs.

Preserve stable IDs, game/engine/scene/entry, character identity, raw/display/localized speaker fields, raw/display/spoken Japanese, localization, voice IDs and root-relative paths, duration, source archives/scripts, neighboring lines, presence/expression/pose/BGM/SFX context when available, adult provenance, multi-voice status, audio class, and audit metadata. Missing or ambiguous material stays explicit; no guessed replacement speakers or audio.

Keep speech, reference, and reaction uses independently selectable. A split derivative can have a different use from its parent without destroying either. Preserve expressive speech, laughter, whispers, sleepy delivery, onomatopoeia, and useful reactions.

Measure duration, leading/trailing/internal silence, silence regions/ratios, RMS, peaks, and clipping before choosing thresholds. Treat internal silence as potentially meaningful prosody. Add review tools for playback/looping, source text/localization/context, metrics, excerpt splits, tags, overrides, revision history, and export. Unflagged material can be provisionally usable; explicit manual decisions override automatic policy.

**Track increments:** verified canonical import; reviewable decisions and derivatives; portable audited exports. Current tools remain wherever they are maintained until moving them provides a concrete benefit.

### T2 — Voice experiments and deployment

**Feeds:** A03 local playback, A06 live speech, A10 voice messages, A13 packaging.

Continue the current GPT-SoVITS v2ProPlus experiment. Treat that as a candidate backend, not the core's identity. Preserve the inventory's other candidates as alternatives to revisit only when a measured deficiency warrants it: GPT-SoVITS v2Pro, v4, v5, Fish Audio S2, CosyVoice, and XTTS-v2. This roadmap makes no new claims about their current capabilities or availability.

Keep portable audio, train/holdout lists, reference/reaction manifests, original Japanese transcripts, checksums, a verifier, and machine-specific path materialization. The inventory records approximately 1,400 usable speech clips, a 60-clip holdout, approximately 1,340 training clips/5,963 seconds, and 12 references. These are historical experiment details to verify against current files, not new targets.

Evaluate a zero-shot baseline, smoke training, and selected checkpoints using fixed Japanese sentences, references, and comparable inference settings. Compare identity, pronunciation, artifacts, stability, pacing, and expressive range. The inventory's proposed schedule was SoVITS/GPT smoke epochs 2/3, then SoVITS 8 with S4/S8 and GPT 15 with G5/G10/G15 comparisons. Preserve it as an experiment record; do not restart or override current training from this roadmap. Latest is not automatically best. Keep zero-shot or earlier checkpoints if they win.

Build a reference bank and a human-assisted expression atlas: neutral/deadpan, soft/content, playful/teasing, embarrassed/flustered, sulking/annoyed/angry, concerned, sleepy, excited/surprised, sad, affectionate, and other discovered deliveries. Candidate metadata includes valence, arousal, intensity, pace, voice quality, and reference usability. Technical audio rankings must not masquerade as emotion annotations.

Separate three deployment jobs:

| Job | Candidate approach | Evidence needed |
| --- | --- | --- |
| Training | Temporary private GPU worker; the inventory considered AWS, private S3, encrypted storage, and an A10G/g5.xlarge-class candidate | Reproducibility, successful verified export, cost controls, worker shutdown |
| Batch production | Pinned notebook/worker; optionally Colab + Drive with weights, references, requests, outputs, and metadata | Resumable request IDs, valid outputs, verified archive, recoverable partial jobs |
| Live speech | Measure local inference on the stated RTX 3050 6 GB system first; another adapter if needed | Load time, VRAM/RAM, first-audio latency, real-time factor, quality, failure behavior |

The optional Colab queue accepts text, language, and reference/delivery requests; a helper processes pending requests and archives audio with metadata. rclone/Drive can transport requests/results without inbound access to the home machine. The notebook VM remains disposable. A temporary notebook is not a promised always-on low-latency backend. Keep training endpoints private, pin experiment versions, verify local checkpoints/results, and terminate rented workers after use.

**Track increments:** a comparable listening result; a portable usable model/reference package; a measured provider adapter. Training success is not required to finish A01/A02.

### T3 — Character, animation, and dialogue production

**Feeds:** every visible adventure, especially A01, A03, A06, A10, A13.

Build the Character Bible incrementally: true chibi Vanilla, classic maid outfit, the earlier/pre-Vol.4 visual feel, canonical colors/appearance, speech style, mannerisms, patience, playfulness, affection/conflict style, curiosity, independence, preferences, and things the system must not invent. Separate canonical guidance from learned relationship history.

Grow semantic animation packs and a direct-play reaction bank: sniffing, laughter, breath, sleepy sounds, surprise, small cries, exertion, hesitation, and onomatopoeia. Body actions cover gaze, ears/tail, approach/retreat, sitting, sleeping, sniffing, eating, playing, and touch responses as assets become available.

Develop the offline dialogue factory only as needed: scenario matrix, candidate generation, character filters, semantic duplicate checks, human review, batch TTS, then pack export. Records can carry text/audio, intent, emotion/intensity/tone, conditions/exclusions, cooldown, semantic family, animation, and weight. Scenarios grow to include time, session length, mood/energy, wins/losses where an activity provides them, ignored interactions, repeated touch, idle periods, music, weather, place return, and routine anomalies.

Later, opt-in live-generation archives can propose reusable lines. Review and remove context-specific or private details before promotion. Thousands of reactions are a long-term content ambition, not an early engineering requirement. Content review remains necessary even when generation is automated.

**Track increments:** a minimal coherent visual pack; a reviewed first voice pack; an expandable production workflow; broader coverage and rare content.

### T4 — Rust learning

Rust Quest remains a separate offline terminal learning project for Arch Linux, Neovim, and the terminal. Its normal play uses no API, LLM, browser, or internet. It can use a non-Rust shell initially, Rust exercises, XP/mastery/progression, failure telemetry, and later adaptive quest packs created after reports are reviewed. Rewriting parts in Rust is a later learning option.

Align study with the current adventure: enums and structs for state; traits for senses/providers; Option or explicit confidence types for uncertainty; Result for failures; collections for memory; channels for signals; serialization for persistence; async work for providers; FFI/IPC for platform boundaries. Vanilla-themed exercises do not silently become production code, and completing the entire course does not gate the companion.

## 7. A purposeful polyglot design

The following is a responsibility proposal, not a framework selection.

| Area | Starting language role | Boundary to preserve |
| --- | --- | --- |
| State, clock, events, behavior, cognition, permissions, persistence | Rust as the meaningful portable core | Platform-neutral rules and explicit ownership |
| Corpus processing, analysis, review/export, experiment orchestration, batch content | Python | Reproducible inputs/outputs and provenance |
| TTS and other model execution | Python when the chosen ecosystem benefits from it | Replaceable requests/results; no ownership of character state |
| Desktop host and renderer | Decide during A01; use Rust where suitable | Semantic actions/state views, independent of frame files |
| Android host/UI | Decide during A08; Kotlin/Java or another justified approach | Native lifecycle responsibilities; shared core rules |
| Debug/review interfaces | Choose the smallest tool that serves the workflow | Developer visibility without duplicating simulation logic |
| Content packs and contracts | Versioned declarative data; format chosen when needed | Data does not require rebuilding the engine for every new item |

TypeScript, C/C++, or other languages are welcome only when a concrete interface, platform, or existing library warrants them. “Polyglot” does not mean choosing one language per organ.

Start with simple module boundaries inside a small application. Split a process when platform ownership, model dependencies, isolation, or deployment requires it. Choose among an in-process interface, FFI, subprocess messages, or a network protocol at that boundary's first real use.

A possible repository organization, created gradually, is `docs/`, `core/`, `desktop/`, `mobile/`, `services/`, `tools/`, `schemas/`, and public sample packs. These are organizational candidates; empty directories and speculative service scaffolds are not deliverables. Keep existing corpus/training projects separate until integration or relocation is justified.

## 8. Decisions made when evidence is available

| Decision | When it becomes necessary | Evidence or small experiment |
| --- | --- | --- |
| Desktop rendering, input, stacking | A01 | Actual compositor/window-manager behavior and an interactive prototype |
| Needs, mood variables, activity selection | A02; retune A05 | Short play sessions, understandable choices, no tedious care loop |
| Persistence and elapsed-time semantics | A02 | Restart, suspend, long absence, interrupted save |
| Spine complexity and observation uncertainty | A04 | Sleeping-lift, startle, stale-observation, and clock-correction scenes |
| Memory horizons/types, retrieval and forgetting | A05 | Useful recall, mistaken belief correction, bounded growth |
| Relationship/routine/preference dynamics | A05 | Meaningful continuity over repeated sessions and accelerated-time inspection |
| TTS winner, references/emotion taxonomy, local feasibility, cloud/batch options | T2; integrate A03/A06 | Controlled listening plus resource/latency measurements |
| LLM provider, live-generation amount, archive promotion | A06/T3 | Character consistency, response quality, cost/latency, private-context handling |
| Sensors, local abstraction, place familiarity/decay | A07; phone inputs A08/A09 | User value, permission boundaries, uncertainty and retention |
| Android technology and communication boundary | A08 | Small connected room using real core state |
| Authority device/phone, conflicts, offline policy | A09 | Lifecycle/battery tests, duplicate/stale commands, host failure |
| Notifications, fictional apps, image consistency | A10 | Useful communication without annoyance; recognizable gallery outputs |
| Dream effects, social activity, curated sources | A11 | Coherent causes for behavior without overwhelming the user |
| Shared-world scheduling and multi-character model | A12 | Two-character interaction and resource contention |
| Extent of generalization and public release | A12/A13 or earlier chosen release | A working independent sample pack and clear private-content separation |

Record consequential decisions with the question, evidence, chosen option, alternatives, consequences, and a trigger for revisiting. A small architecture decision record is enough. An experiment should end with a decision or a named unresolved limitation, not permanent speculative infrastructure.

## 9. How adventures become implementation work

Before starting an adventure, create a short adventure plan containing its user experience, acceptance scene, dependencies, goal order, current decisions, and explicit boundaries. Expand only the first ready goal into missions.

Each mission brief should state the observable result, relevant current code, allowed scope, important contracts, implementation steps, and checks needed to establish that result. Interface details and exact commands belong there, after inspecting the real repository.

For example, the first planning session after this roadmap would develop **A01**, in this order:

1. Describe the desktop behavior and constraints on the user's actual machine.
2. Run the smallest rendering/input experiment needed to choose the host approach.
3. Define the first semantic animation and pointer interaction slice.
4. Prepare the initial implementation mission and its visible acceptance check.

The milestone is the running companion, not completion of the experiment alone.

An adventure is complete when:

- Its acceptance scene works on the target machine and earlier supported behavior remains usable.
- Its meaningful state survives restart where applicable; provider-dependent features handle unavailability clearly.
- It has just enough diagnostics to explain its new behavior and failures.
- The user can launch it from documented instructions and keep using it between adventures.
- The relevant decisions, known limitations, and progress are recorded; a stable checkpoint/tag can be made.

Verification grows with risk: visible interaction checks early, state-transition/save checks when persistence arrives, simulated signal checks for cognition, failure/reconnect checks for devices, and listening comparisons for voice. We do not build a large testing framework before the first body. Long-term experience also needs ordinary use between increments.

No final asset collection, massive dialogue corpus, chosen cloud provider, or permanent microservice architecture is required to start programming.

## 10. Coverage of the original inventory

Every numbered source section has a home below. A mapping preserves scope; it does not claim the idea is implemented or mandate every example. Alternatives and historical experiments remain in their supporting track or decision point.

| Source § | Idea | Roadmap home |
| --- | --- | --- |
| 1 | Persistent character vision; private implementation and generic engine | Principles; A02–A13 |
| 2 | One Vanilla, multiple bodies; possible phone authority | A08, A09 |
| 3 | Brain, world, Spine, senses, effectors, services, hosts | Principles; A01–A09; polyglot design |
| 4 | LLM as tool, simulation as durable authority | Principles; A06 |
| 5 | Truth, observation, belief, memory, metacognition | A04, A05, A06 |
| 6 | Uncertain, incomplete, stale senses | A04, A07 |
| 7 | Spine, reflexes, escalation, autonomic reactions | A04 |
| 8 | Sleep changes perception, wake thresholds, memory eligibility | A04 |
| 9 | Memory horizons, functions, retrieval, consolidation, forgetting | A05 |
| 10 | Slow relationship state distinct from mood | A05 |
| 11 | Physiology, care activities, item properties | A02; preferences A05 |
| 12 | Mood dimensions, dynamics, complex expressive delivery | A02, A05, A06; T2 |
| 13 | Agency, utility candidate, behavior explanations | A02, A05 |
| 14 | Silence and nonverbal behavior | Principles; A01–A06; A13 |
| 15 | Arch/bspwm desktop body and everyday actions | A01, A02 |
| 16 | Semantic animation names and metadata | A01, A03; T3 |
| 17 | Cursor as hand; escalating interactions | A01, A02; A13 |
| 18 | True chibi, classic outfit, Character Bible | T3; A01, A03 |
| 19 | Offline dialogue first; live generation for novelty | A03, A06; T3 |
| 20 | Dialogue metadata and contextual conditions | A03; T3 |
| 21 | Offline generation, filtering, review, batch TTS | T3 supported by T2 |
| 22 | Live dialogue and possible sentence streaming | A06 |
| 23 | Language-to-semantic-event interpretation | A06 |
| 24 | Personal routines, statistics, anomalies | A05 |
| 25 | Rare events and hidden long cooldowns | A05, A11, A13; T3 |
| 26 | Android room, lifecycle, efficient persistent core | A08, A09 |
| 27 | Sync, ordering, conflicts, versions, migrations | A08, A09 |
| 28 | Text/voice/picture messages, delay, anti-annoyance | A10 |
| 29 | Permission, local raw processing, retention, fictional abstraction | A07; A06, A08–A10 |
| 30 | Anonymous places, scent/familiarity, user-taught labels | A07; Android adapters A08/A09 |
| 31 | Weather, time, return, charging, availability | A07; reflex connection A04 |
| 32 | Fictional phone apps and authorized capabilities | A10, A11 |
| 33 | Semantic picture requests, continuity, gallery memories | A10; T3 |
| 34 | Nekogram posts, accounts, reactions, threads | A11 |
| 35 | Controlled real information, structured events | A07, A11 |
| 36 | Dreams and autonomous narrative | A11 |
| 37 | Multiple characters, one authoritative shared world | A12; T1 |
| 38 | Public engine and private character packs | Principles; A13; pack proof A12 |
| 39 | Meaningful Rust core and language-concept mappings | Polyglot design; T4 |
| 40 | Separate offline Rust Quest and adaptive learning reports | T4 |
| 41 | Exact source/voice corpus, engine adapters, all-character context | T1 |
| 42 | Rich canonical fields and uses beyond TTS | T1; T3 |
| 43 | Immutable originals, provenance, auditable normalization | T1 |
| 44 | Speech/reference/reaction uses and split derivatives | T1, T2, T3 |
| 45 | Preserve expressive audio | T1, T2 |
| 46 | Distribution-led silence/audio measurement | T1 |
| 47 | Human review, context, overrides, excerpt history | T1 |
| 48 | GPT-SoVITS variants and alternative TTS candidates | T2; evidence-gated choices |
| 49 | Reference-conditioned delivery and reference bank | T2; A06 |
| 50 | Human-assisted expression atlas and delivery dimensions | T2, T3 |
| 51 | Stable replaceable voice service | A03, A06; T2 |
| 52 | Portable first experiment package and historical counts | T2; verify current artifacts before reuse |
| 53 | Controlled baseline, smoke, checkpoints, listening comparison | T2 |
| 54 | Private temporary AWS/cloud training and verified export | T2 |
| 55 | Colab/Drive batch generation and persistent artifacts | T2 |
| 56 | Pending generation requests and output metadata | T2 |
| 57 | rclone transport/archive without home inbound access | T2 |
| 58 | Disposable batch workers, no always-on Colab dependency | T2; A06 fallback |
| 59 | RTX 3050 local inference measurements | T2 |
| 60 | Direct-play nonverbal reaction library | A03; T1, T3 |
| 61 | Generated-audio archive and reviewed promotion | A06; T3 |
| 62 | Voice, expression, animation, interruption coordination | A03, A06 |
| 63 | Typed world/body/behavior/service/device events | A02, A04, A06, A08; polyglot design |
| 64 | Simulation clock, elapsed time, subjective time, scheduling | A02, A04, A05, A09, A10 |
| 65 | State, belief, memory, behavior, voice, sync diagnostics | Every adventure as its system appears |
| 66 | Hide machinery, improve pacing, silence, refusal, subtlety | A13; incremental polish throughout |
| 67 | First playable scope and excluded later systems | A01–A03 stopping point |
| 68 | Repository modules and private/local data separation | Polyglot design; T1/T2; A13 |
| 69 | Private source assets/models/packages and sensor permissions | Principles; T1/T2; A07; A13 |
| 70 | Replaceable LLM/TTS/image/cloud/device adapters | Polyglot design; A03, A06, A08–A10 |
| 71 | System knowledge exceeds character knowledge | Principles; A04–A07, A10, A11 |
| 72 | Autonomous offline life without constant AI | Principles; A02–A05; A11 |
| 73 | Open design questions | Decision register; individual adventure choices |
| 74 | Overall interconnected architecture | Adventure dependencies; principles; polyglot design |
| 75 | Coherent continuity and milestone playability | Destination; completion rules; entire route |

## 11. Updating this roadmap

Keep the inventory as the idea record and this roadmap as the current route. As evidence arrives, a goal may move, split, merge, or be deferred. Preserve its coverage entry and record the reason so ideas do not disappear through editing.

Keep adventure and goal IDs stable once work starts. A small active-adventure document can hold implementation detail without inflating this roadmap. Maintain a brief progress record containing the working checkpoint, completed goals, active mission, known limitations, and next decision.

The next concrete step is to design A01 against the user's machine and prepare its first programming mission. Its success is a character on the desktop that can already be played with.
