# UX Task-Series Parity Report

Generated 2026-08-09T21:03:55.214Z by e2e/parity-tasks.mjs.
Both servers serve the SAME live DB — every divergence is a server gap.

| Step | Python (oracle) | Rust | Verdict |
|---|---|---|---|
| A.session-list | `{"archivedCount":65,"previewLinesShapes":["array-of-strings"],"probes":{"amux":{"archived":false,"flags":true,"hasPreview":true,"previewLinesShape":"array-of-st` | `{"archivedCount":65,"previewLinesShapes":["number"],"probes":{"amux":{"archived":false,"flags":true,"hasPreview":false,"previewLinesShape":"number","status":"",` | DIVERGES — facts differ |
| B.board-data | `{"byStatus":{"armed":3,"backlog":536,"blocked":10,"discarded":27,"doing":8,"done":67,"needsyou":19,"review":77,"todo":37,"verified":141},"fields":["archived","c` | `{"byStatus":{"armed":3,"backlog":537,"blocked":10,"discarded":27,"doing":8,"done":65,"needsyou":19,"review":76,"todo":36,"verified":8},"fields":["archived","cre` | DIVERGES — facts differ |
| C.groups | `{"admin":1,"amux":7,"canvas":5,"customers":10,"gtm":11,"ip-check":1,"mvs":3,"new-features":4,"ops":12,"personal":8,"plg":3,"public-assets":4,"sherpa":2,"testpai` | `{"admin":1,"amux":7,"canvas":5,"customers":10,"gtm":11,"ip-check":1,"mvs":3,"new-features":4,"ops":12,"personal":8,"plg":3,"public-assets":4,"sherpa":2,"testpai` | PARITY |
| D.board-write-flow | `{"create":201,"createdId":"yes","edit":200,"move":409}` | `{"create":201,"createdId":"yes","edit":200,"move":409}` | PARITY |
| E.schedules | `{"count":111,"enabled":20,"fields":["command","created","deleted","done_action","done_pattern","enabled","exit_actions","fires_day","fleet_share","gcal_event_id` | `{"count":111,"enabled":20,"fields":["command","computed_next_run","created","deleted","done_action","done_pattern","enabled","exit_actions","fires_day","fleet_s` | PARITY — rust-only extras (parity): computed_next_run |
| F.calendar | `{"count":78,"status":200}` | `{"count":78,"status":200}` | PARITY |
| G.settings-backends | `{"prefsCount":72,"usage":200,"usageHasWindows":true}` | `{"prefsCount":72,"usage":200,"usageHasWindows":true}` | PARITY |
| H.session-verb-peek | `{"hasOutput":true,"status":200}` | `{"hasOutput":true,"status":200}` | PARITY |
| H2.tab-endpoints | `{"map":200,"mapHasSettings":true,"mapPins":68,"skillFields":["description","hint","name"],"skills":200,"skillsCount":9,"slash":200,"slashCount":70,"statusesIds"` | `{"map":200,"mapHasSettings":false,"mapPins":"absent","skillFields":[],"skills":200,"skillsCount":"non-array","slash":200,"slashCount":"non-array","statusesIds":` | DIVERGES — facts differ |
| J.crm-tab | `{"visibleCrmTab":false}` | `{"visibleCrmTab":false}` | PARITY — python shows it: false (intended difference) |
| I.worker-tab-numbers | `{"boardChips":["Working now36","Needs you157","Armed9","Unowned12","Mine24","⚡ Focus157"],"sessionTabs":["🔔0","49","0"]}` | `{"boardChips":["Needs you72","Armed9","Rotting60","Unowned12","Mine24","⚡ Focus72"],"sessionTabs":["🔔0","49","0"]}` | DIVERGES |

4 step(s) diverge — see rows above.
