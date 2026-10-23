# D — Moves: teaching and learning skills

Raising your own partners means each one learns your repeated work as named **moves**. Moves belong to the individual partner, four per form: Bolt's Jolteon moves come back whenever Bolt becomes Jolteon again, and another partner doesn't know them until taught (MV08A). A move is a single-form reusable action; an Evolution Stone (S03 recipe) stays a whole-task workflow. Moves reuse S03's step types and validation, carry no grants, and every use goes through the normal review.

Teaching, use, learning, reminders and direct party sharing are complete; their specifications and evidence are in [COMPLETED.md](COMPLETED.md). Only portable TM files remain here.

| Form | Move kind | Example default name |
|---|---|---|
| Eevee | task template (goal + criteria) | Helping Hand |
| Vaporeon | pin set (paths/ranges resolved at use) | Surf |
| Jolteon | run (exact argv + project-relative cwd) | Thunderbolt |
| Umbreon, Leafeon | run through Jolteon's review (diagnostic/measurement) | Foul Play, Leaf Blade |
| Espeon, Flareon, Glaceon, Sylveon | request template (instruction + context selectors) | Future Sight, Flamethrower, Ice Beam, Moonblast |

### MV08B — TM export and import

- [ ] Share moves across projects and people as files.
- **Priority:** Pokémon milestone 2. **Needs:** —. **Builds on:** MV01A, MV01B, MV01C, MV02B, MV04, MV08A, S03, S01.
- **Existing foundation:** Typed moves, four slots per partner/form, local teaching and fresh use-time reviews already exist. Database records contain local identities and are not a portable format.
- **Change:** Export a bounded, versioned TOML TM definition containing only portable move semantics. Reuse existing create-only safe publication and step validation; implement the missing reviewed portable-export slice here so S04 can later reuse it. Full workflow extraction in S04 is not a prerequisite. Import previews name, kind/steps, form, chosen local partner, scope and free/replacement slot, then commits inert local learning through the existing writer.
- **Accept:** Round-trip every supported move kind between projects and across restart. Exercise name collisions, full-slot replacement, cancellation, unsupported versions, unsafe paths, recognizable secrets and an already-existing output file. Export never overwrites. Imported pin selectors may be unavailable until use; missing files are shown honestly, never substituted or automatically captured. Using the move still requires fresh ordinary review.
- **Watch out:** Never deserialize foreign task/partner/source IDs as local provenance or invent prior success, teacher authority or evidence. Explicit local learning and its existing milestone rules remain separate from imported claims. Import never sends, executes, selects context or restores approvals/sessions.
