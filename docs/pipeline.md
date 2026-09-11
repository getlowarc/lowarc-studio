# The pipeline: source to screen

**Status: TWO MODELS ARE ON THE TABLE. Read the next section before trusting stages 4 and 5.**
Written down mid-design so it stops having to be re-derived. Each stage says whether it exists today;
nothing here describes shipped behaviour unless it says so.

## Two models, and the second is the one LowArc started from

Everything below stage 3 was written for the **translation model**, and there is a second model that
overturns most of it. Neither has been chosen.

**A · Translation.** An interpreter module reads the user's language and rewrites the code inside the
build copy into a form the other modules work with. Stages 4 and 5 below describe this.

**B · Native execution: the original idea.** The user's code runs normally, in its own language's
runtime. Modules **provide** the words (`rectangle`, `play`) as real functions in that runtime; the
code calls them because they exist. Nothing is parsed, translated or mutated by LowArc. Only the part
of the code written *for* the modules ever crosses the boundary: logic, physics, control flow and
variables just run, and LowArc never sees them.

Model B was how LowArc was originally conceived, and it is recorded here because it is stronger than
it first looks:

- **It dissolves the open question at stage 4 rather than answering it.** `let hit = raycast(a, b)` is
  an ordinary function call in an ordinary runtime.
- **No format needs designing.** Arguments arrive already shaped the way the module expects, because
  the module defined the signature when it defined the function.
- **Imports come free.** The language's own module system already resolves them, so stage 4's
  tree-walking and per-file interpreter selection are unnecessary.
- **It answers the speed requirement structurally.** LowArc's overhead becomes proportional to the
  *number of module calls*, not to the size or speed of the user's code. It cannot slow down what it
  never touches.
- **It removes LowArc as a possible author of bugs**, which is one of the two things that must never
  happen: no translation means no mistranslation.

What model B costs: an interpreter is replaced by a **host** per language. Something that starts
that runtime, puts the modules' words in front of it and drives frames. A host is a fraction of an
interpreter (no parsing, no rewriting, no source maps), and it is one per *language*, not per module,
provided modules expose a uniform ABI, which already exists as the JSON line protocol and the C ABI.
Compiled languages are the awkward case: bindings cannot be injected at runtime, so the host becomes
a build step plus a library to link.

What model B gives up: anything clever at compile. No static checking, no LowArc understanding the
code before it runs. Studio's debugger would watch the module boundary rather than the user's own
logic: solvable with a module for it, but a real change in what the IDE can see.

**What survives under either model:** source is never touched and both dev and export run a copy; the
vocabulary belongs to the modules and never to LowArc; and state-in / calls-out, where emit-only
calls can be buffered and only genuine queries need an answer back.

**Provide, never scan.** Under model B a module supplies its words as real functions. It does not
search the source for known names: an alias or a wrapper defeats that, and it would put LowArc back
to guessing at code it does not parse.

## The invariant

**A user's code can be written however they like: any language, any format.** They conform to the
framework their chosen modules define, and to nothing else. Everything below is subordinate to this.

Consequences, all of them downstream of that one line:

- LowArc has no language of its own and no project format.
- The entry file is an **entry point, not the only file**. A project is a source tree.
- **An interpreter module understands one language and nothing else.** Syntax, with no opinion about
  what the code is *for*: that is what makes it reusable across a game, a tool, or something with no
  screen at all. A separate module decides what the translated code means.
- **The vocabulary belongs to the modules.** Install a draw module that names it `rect` and you write
  `rect`. LowArc never has an opinion about the name.

Module dilution is a lesser goal. Fusing modules, user code and engine into one artifact is
desirable, not sacred; where it conflicts with the invariant, the invariant wins.

## Two rules that constrain every stage

**Source is never touched.** Dev run and export both work on a *copy*. The copy a dev run executes is
the same artifact an export would produce; they differ only in what happens after it is built.

**LowArc must never make the code slower than it has to be, and must never be the author of a bug.**
The more the pipeline rewrites, the more surface there is for LowArc to be at fault. This is why
"rewrite everything for elegance" is not on the table.

---

# The pipeline

## 1 · Open a project: *exists*

The IDE opens a root directory. The runtime knows it too: `start_run` takes `project_dir` alongside
`entry_file`.

## 2 · Build the copy: *decided, not implemented*

Both dev run and export copy the project first, and run or package **the copy**. Rebuilt on every dev
run and scrapped when it ends: no incremental build, no cache to invalidate, no stale-artifact class
of bug.

A copy identical to an export contains the assets, and copying hundreds of megabytes of textures on
every Run would be felt, so **link rather than copy anything the build does not modify**.
Near-instant on NTFS, and byte-identical to what an export produces.

*Today's export instead stages: it copies the source, copies each module folder, writes a
`launch.json`, and drops the runtime exe beside it. Engine, modules and source stay three separate
things. The copy-based model is new work in both paths.*

## 3 · Resolve and order the modules: *exists*

`project.json`'s `requires` is walked to a dependency closure against the module store, then sorted
so a provider always runs before its consumers. Contract modules resolve here and are never spawned.

## 4 · Compile: *exists, but does almost nothing yet*

This is where the real work belongs. Four things happen:

1. **Walk the tree.** Universal, so it belongs to the runtime: every interpreter reimplementing
   directory traversal is waste.
2. **Select an interpreter per file**, by the file types it publicly declares. Precedent for the
   shape: a plugin already declares `contributes.viewers[].extensions` in `plugin.json`.
3. **Follow imports.** What a file imports is syntax, so the interpreter reports it; the runtime
   resolves the path and serves the bytes. Resolution *policy* is language-specific: `node_modules`,
   Python packages and Rust crates follow different rules, so the runtime must not pretend to own
   it. Start at the entry, pick its interpreter, let it report what it imported, resolve, recurse.
   Cross-language imports fall out for free.
4. **Mutate the copy.** Rather than producing a message, an interpreter **rewrites the user's code
   inside the build copy** into a form the modules downstream can work with. The artifact is a file
   on disk (openable, diffable, cacheable), and it is the speed win: rewritten once at compile, so
   nothing re-parses it sixty times a second.

Two things travel with the rewrite: a **format** still has to exist (it moved from the wire to the
disk, it did not vanish), and **source maps**, because once the running code is not the code the user
typed, an error must still point at their line.

> ### ⟵ THE OPEN QUESTION LIVES HERE
>
> **What does the mutated code look like, such that one call's result can feed what comes after it?**
>
> A call is a name and its arguments: settled. What is unsettled is how the rewritten form expresses
> *"this step needs that step's result."*
>
> **Why it lives at compile and not at run:** it is tempting to frame this as "execution has to stop
> mid-statement waiting for an answer," as in `let hit = raycast(a, b)`. That framing imports a
> constraint from a language that no longer exists by the time anything runs. The source was consumed
> at compile; what executes is the mutated form, and there may be no statement to stop inside. The
> question is about the **shape of that form**, not about a running process blocking.
>
> Ordinary shapes exist for it: results bound to slots that later steps reference; the code split
> into steps at each call boundary; a form where whatever walks it resolves calls as it goes. All of
> them dissolve "blocking mid-statement", because there are no statements left: there are steps.
>
> This is also one of the most well-trodden problems in compilers. `async`/`await` and generators are
> precisely this: a compiler splitting a function at the points where it must wait and rebuilding it
> as something resumable. The technique does not need inventing, only choosing.
>
> **It blocks nothing.** Calls whose results feed later steps are the minority; state-in / calls-out
> covers a rectangle on screen and most of what follows.
>
> **One division worth not getting backwards, for when the runtime side is built:** the *vocabulary*
> is the modules' business and LowArc must never have an opinion about it: that is what a contract
> is for. The *transport* is LowArc's, because LowArc owns the only pipe: modules are separate OS
> processes and the runtime holds every stdin/stdout handle, so "the modules will agree among
> themselves" is not possible even in principle. They have nothing to agree over. A module could
> open its own socket, but then module authors reinvent discovery, handshakes, lifetimes and error
> handling, and none of the runtime's guarantees apply: not ordering, not stop, not degraded
> reporting, not breakpoints, not the frame trace.

*Today: every module receives `{sourceCode, sourcePath}` and replies `{ok, items}`; those items are
handed back to that module at start. Nothing puts anything in `items` or reads them. A module
answering `ok: false` is dropped from the run.*

## 5 · The frame loop: *exists*

Per tick, in run order:

**a. State in.** The runtime hands each module `{delta, shared}`, filtered to what it `requires`. The
interpreter is therefore *holding* everything any module published (`input-state` included) before
the user's code runs.

**b. The interpreter executes this frame's code.** Not a translation that walked away: control flow
means it has to run the code to know which calls happen.

- A read (`mouse.x`) is a **lookup** in the state it is already holding. No call, no cost.
- A call (`rectangle(x, y, w, h)`) is recorded as **a name and its arguments**: the interpreter has
  no idea what `rectangle` means.

Note that the code being executed here is the **mutated** form, not the user's source. Whatever the
source language could or could not express stopped mattering at stage 4; how a call's result reaches
a later step is decided by the shape chosen there, not by anything at this stage.

**c. Calls out.** The interpreter publishes this frame's calls.

**d. The module that knows what the names mean** runs next, recognises `rectangle`, and maps it onto
the output's vocabulary: `draw-commands`, or whatever the output speaks.

**e. The output module renders.** `vector-canvas` gathers `draw-commands` from every provider in run
order and draws them; draw order is list order, so run order is z-order. It publishes `input-state`
back for the next tick.

*Execution is sequential and deliberately so. Run order is what guarantees a consumer sees this
tick's output rather than last tick's. Running independent modules in parallel is possible and the
dependency graph to identify them exists, but the gain is bounded by the longest chain, and this
pipeline is a chain.*

## 6 · Stop: *exists*

Any module can send `{"requestStop": true}` to end the run. Every module gets its `stop` phase. The
dev copy is then scrapped.

## Export: *decided, not implemented*

Identical through stage 4, then packaged instead of run. Same copy, same compile, same artifact,
which is what makes "it worked in dev" mean something.

---

## What compiling buys, precisely

Compiling makes things faster in exactly one way: **it moves work from every frame to once.** It does
not on its own remove the cost of separate programs talking: that only goes away by removing the
separateness, which is module fusion, which is the lesser goal. So "compile it together and it gets
faster" is half true, and stage 4's rewrite is the half worth building.

## Whether the per-frame protocol is fast enough is a measurement

One JSON object per module per frame over stdin/stdout: at 60fps with four modules, 240
write-flush-read-parse round trips a second before any of the user's own work. Whether that is
acceptable is measurable, not arguable: `FrameModuleTrace` already records `duration_ms` per module
per frame, and nothing has yet looked at it for this question.
