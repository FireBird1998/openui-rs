# 150 recorded streams: the requests

Testing ticket 11: **150 requests**, comprising **120 fresh screens and 30 follow-ups**, split equally between Sonnet and Haiku. Record in the viewer with the **react-ui** library (the default), at **low** effort, with the model shown. Follow-ups (41–50 and 131–150) use the fresh request in their **Of** column in edit mode, just like **Ask for a change**.

The maintainer can record the list sequentially with batch mode. First save the API key through the viewer's Settings (macOS Keychain), confirm the budget cap and recordings folder there, and build the app. From the repository root:

```sh
scripts/viewer.sh build
VIEWER_RECORD="$PWD/viewer/Requests/recorded-streams.json" target/viewer/debug/Viewer.app/Contents/MacOS/Viewer
```

Run the same command again to resume: completed batch recordings with matching request metadata are skipped. To select an inclusive range, for example the new requests:

```sh
VIEWER_RECORD="$PWD/viewer/Requests/recorded-streams.json" VIEWER_RECORD_FROM=51 VIEWER_RECORD_TO=150 target/viewer/debug/Viewer.app/Contents/MacOS/Viewer
```

Follow-ups use their named parent's program from this run or its saved batch recording, even when the parent is outside the selected range; unavailable parents are skipped with a reason. Keep the full JSON list when selecting a range. Manual recordings have no batch identity and are not skipped automatically. Partial/failed streams are saved when they contain text, but retried on resume. Streams stopped at the output-token limit are saved and count as recorded, so their truncation can be investigated.

The app prints one result line per selected request, then totals and estimated spend for this run, and quits. It stops sending after three consecutive failed attempts (skips do not reset the count), or before a request's conservative cost reservation would exceed the existing budget cap. Reservations allow the full output limit, prompt and follow-up bytes, cache-write overhead and the highest configured fallback price; completed usage reconciles the estimate. Interrupted/failed requests with unknown usage retain their reservation in Settings. This can stop a run short of $10; inspect spending before adjusting it. The estimate uses the viewer's configured prices, not a provider billing guarantee. Batch mode uses the same Keychain access as the sheet and never logs the key or raw server errors.

After recording:

```sh
scripts/import-recordings.sh
# If Settings uses a custom recordings folder:
# scripts/import-recordings.sh "/path/to/recordings"
```

The machine-readable list lives in `viewer/Requests/recorded-streams.json`, outside the conformance corpus. Regenerate it after editing these tables with `python3 viewer/Requests/sync-recorded-streams.py`; `scripts/check-viewer.sh` checks that all 150 rows agree, including numbering, models and follow-up parents.

**Estimated spend:** about $3.30 in all, at the viewer's prices and its prompt (about 9,500 tokens): roughly 4 cents per Sonnet 5.5 request and 0.2 cents per Haiku 5.5 one, with outputs of 1,500 to 2,000 tokens. Well inside the $10 cap, which the viewer enforces.

## First 40 fresh screens (1–40)

About half are news-shaped, as a preview of Dynamic News.

| # | Model | Request |
|---:|---|---|
| 1 | Sonnet | Front page for a local news site: a lead story with an image, three secondary headlines, and a "most read" list. |
| 2 | Haiku | Breaking news banner about a city-wide power outage, with the latest update time and a link to live coverage. |
| 3 | Sonnet | A live blog of a product launch event with six timestamped updates, newest first. |
| 4 | Haiku | Weekend weather for three cities as cards, each with a high, a low and a short forecast. |
| 5 | Sonnet | Election night results for a mayoral race: candidates, vote share as a bar chart, and precincts reporting. |
| 6 | Haiku | A sports scoreboard for tonight's four basketball games, with the final and in-progress ones marked. |
| 7 | Sonnet | Market summary: three index values with daily change, a line chart of the week, and top movers in a table. |
| 8 | Haiku | An article page: headline, byline, publish date, lead image, four paragraphs and related links. |
| 9 | Sonnet | A newsletter digest with five stories, each a title, a one-line summary and a "read more" button. |
| 10 | Haiku | Traffic alerts for the morning commute: three incidents with severity tags and estimated delays. |
| 11 | Sonnet | A fact-check card rating a viral claim as "mostly false", with the claim, the verdict and three sources. |
| 12 | Haiku | Top headlines by section, in tabs for World, Business, Tech and Sport, three per tab. |
| 13 | Sonnet | An explainer about a new transit fare system, in an accordion of five questions and answers. |
| 14 | Haiku | Storm warning for the coast: alert level, affected areas, timeline steps and safety tips. |
| 15 | Sonnet | Opinion section: three columnists with photos, their latest piece title and a short excerpt. |
| 16 | Haiku | A podcast episode page for a daily news show, with the episode list and a play button for each. |
| 17 | Sonnet | Quarterly earnings for a retailer: revenue and profit as KPIs, a bar chart by quarter and a short analysis. |
| 18 | Haiku | Obituary notice with a photo, dates, a short biography and service details. |
| 19 | Sonnet | Local events this week as a list, each with a date, a venue and a tag for free or ticketed. |
| 20 | Haiku | Corrections and clarifications page listing three recent corrections with the original and corrected text. |
| 21 | Sonnet | A team task board with three columns (to do, doing, done) and cards with assignees. |
| 22 | Haiku | A sign-up form with name, email, a password with rules, a plan select and terms checkbox. |
| 23 | Sonnet | A sales dashboard: four KPI cards, monthly revenue line chart and a table of top products. |
| 24 | Haiku | A recipe card for pancakes with ingredients, numbered steps and prep time. |
| 25 | Sonnet | Settings page with switches for notifications, a theme radio group and a save button. |
| 26 | Haiku | A pricing page with three plans as cards, the middle one highlighted as most popular. |
| 27 | Sonnet | Flight booking search: from, to, dates, passengers, and three result rows with prices. |
| 28 | Haiku | A contact card for a support team with hours, phone, email and a button to start a chat. |
| 29 | Sonnet | Project status report: milestones as steps, a risks table and an overall health tag. |
| 30 | Haiku | A product page for headphones with an image gallery, specs table and add-to-cart button. |
| 31 | Sonnet | An onboarding checklist of five steps with progress and the next step highlighted. |
| 32 | Haiku | A gym class timetable for the week in a table, with class type tags. |
| 33 | Sonnet | A support ticket view: the ticket details, a conversation thread and status actions. |
| 34 | Haiku | An FAQ page for a library: opening hours, membership, borrowing limits, in an accordion. |
| 35 | Sonnet | A personal finance overview: balances, a pie chart of spending categories and recent transactions. |
| 36 | Haiku | A course syllabus with weeks, topics and assignment due dates. |
| 37 | Sonnet | A hiring pipeline: candidates per stage as a bar chart and a table of open roles. |
| 38 | Haiku | A restaurant menu with sections for starters, mains and desserts, with prices. |
| 39 | Sonnet | A server monitoring panel: CPU and memory charts, uptime, and an incidents list. |
| 40 | Haiku | A travel itinerary for three days in Lisbon, each day a card with morning, afternoon and evening plans. |

## First 10 follow-ups (41–50)

Each changes the screen of the fresh request named, in edit mode, with the same model it used.

| # | Model | Of | Request |
|---:|---|---:|---|
| 41 | Sonnet | 1 | Add a weather strip at the top and make the lead story's image larger. |
| 42 | Haiku | 2 | The power is back: change the banner to "Power restored" and add a timeline of the outage. |
| 43 | Sonnet | 5 | Add a second race for city council below the mayor's, with its own chart. |
| 44 | Haiku | 6 | Mark one game as overtime and add a "watch highlights" button to the finished ones. |
| 45 | Sonnet | 7 | Replace the line chart with an area chart and add a currency exchange table. |
| 46 | Haiku | 12 | Add a Science tab with three headlines. |
| 47 | Sonnet | 23 | Add a date range select that filters the dashboard, and remove the products table. |
| 48 | Haiku | 22 | Add a phone number field and make the plan select default to the free plan. |
| 49 | Sonnet | 29 | Mark the second milestone as done and add a new risk. |
| 50 | Haiku | 40 | Swap day two and day three, and add a restaurant suggestion to each evening. |

## Another 80 fresh screens (51–130)

The first 40 in this group are news-shaped; the rest exercise other everyday screens. Use illustrative data; these ask for UI examples, not live research. All can be expressed with the example react-ui library.

| # | Model | Request |
|---:|---|---|
| 51 | Sonnet | A newsroom assignment desk: six pitches with reporter, deadline, reporting stage and an unassigned tag, plus a pitch submission form. |
| 52 | Haiku | A reader's saved-story shelf with unread and finished sections, estimated reading times and remove buttons. |
| 53 | Sonnet | An investigation into reservoir pollution: a timeline of sampling dates, a contaminant-level line chart and cards linking the evidence documents. |
| 54 | Haiku | A newspaper subscription delivery notice: missed delivery dates, current address and a form to request a replacement copy. |
| 55 | Sonnet | Compare three candidates' positions on housing, transport and schools in a table, with a short quote from each and source links. |
| 56 | Haiku | A radio news bulletin rundown with segment durations, presenter names and a total running-time card. |
| 57 | Sonnet | A housing affordability report: median rent by district in a horizontal bar chart, household-income figures and a methodology note. |
| 58 | Haiku | A press conference transcript excerpt with speaker labels, timestamps and a callout highlighting the announced decision. |
| 59 | Sonnet | A reader survey about local coverage: topic checkboxes, preferred edition radio choices and a free-text suggestion field. |
| 60 | Haiku | A photo essay about restoring a historic theatre: four captioned images and a short introduction, followed by photographer credits. |
| 61 | Sonnet | A city budget explainer comparing proposed and approved spending by department in a grouped bar chart and a changes table. |
| 62 | Haiku | A newspaper archive search screen with a keyword field, year select and four dated issue result cards. |
| 63 | Sonnet | A science story comparing two telescope discoveries: observation facts in a table, an image for each and a plain-language glossary. |
| 64 | Haiku | A community noticeboard with lost-and-found notices, volunteer requests and public meetings grouped into separate cards. |
| 65 | Sonnet | A newsroom source directory: five fictional experts with fields of expertise, institution and contact buttons, plus a subject filter. |
| 66 | Haiku | A public hearing coverage page listing agenda items, testimony summaries and the next consultation deadline. |
| 67 | Sonnet | A drought tracker with reservoir levels as a bar chart, weekly rainfall figures and water restriction tags for four towns. |
| 68 | Haiku | An editor's letter explaining a new weekend edition, with three sample sections and a reader feedback button. |
| 69 | Sonnet | A rail service reliability investigation: on-time percentages by operator, monthly cancellation trends and a table of the worst routes. |
| 70 | Haiku | A book review roundup with cover images, reviewer verdicts and short excerpts for four new novels. |
| 71 | Sonnet | A civic data request tracker showing requested documents, agency, submission date and response status, plus overdue notices. |
| 72 | Haiku | A newsroom transparency page listing funding sources, editorial independence rules and how readers can report a conflict of interest. |
| 73 | Sonnet | A local school enrollment report with capacity versus enrollment by school, a district total and notes about planned expansions. |
| 74 | Haiku | A reader letters page with three letters, author towns and a form for submitting a new letter. |
| 75 | Sonnet | A regional broadband coverage report with speed tiers by town in a table and a bar chart of households still unserved. |
| 76 | Haiku | A union negotiation briefing: employer offers and union demands side by side, dates of talks and a list of unresolved items. |
| 77 | Sonnet | A public-health data bulletin with fictional weekly clinic visits, a line chart by week and a note explaining the data lag. |
| 78 | Haiku | A documentary screening review with a poster image, critic's summary, accessibility details and upcoming screening dates. |
| 79 | Sonnet | A newsroom correction request form with article URL, disputed passage, supporting evidence and contact preference fields. |
| 80 | Haiku | A court-case explainer showing the parties, procedural timeline, disputed questions and a glossary of legal terms using fictional details. |
| 81 | Sonnet | A regional energy report with generation mix in a pie chart, monthly demand in a line chart and a planned maintenance notice. |
| 82 | Haiku | A cultural heritage feature with cards for three endangered crafts, artisan quotes and workshop visit details. |
| 83 | Sonnet | A neighborhood amenities comparison: parks, clinics and libraries per district in a table, with a short interpretation and source notes. |
| 84 | Haiku | A community journalism tip form with topic, location, description and a checkbox requesting confidentiality. |
| 85 | Sonnet | A disaster recovery spending tracker with pledged versus disbursed funds by project and completion-status tags. |
| 86 | Haiku | A journalist profile with reporting beats, awards, a short biography and selected investigations displayed as cards. |
| 87 | Sonnet | A coastal erosion report with shoreline retreat measurements by beach, a before-and-after image pair and a monitoring schedule. |
| 88 | Haiku | A media literacy lesson showing a headline, evidence checklist and three questions readers should ask before sharing it. |
| 89 | Sonnet | A public procurement investigation with contract values by supplier, tender dates and callouts for missing disclosures. |
| 90 | Haiku | A bilingual news brief presenting the same fictional museum opening in English and Spanish in two clearly labeled cards. |
| 91 | Sonnet | A seed inventory for a community garden: crop, variety, quantity and sowing month, with low-stock tags and a restock form. |
| 92 | Haiku | A pet adoption directory with four animal portraits, ages, temperament tags and meet-and-greet buttons. |
| 93 | Sonnet | A warehouse receiving checklist with expected shipments, counted quantities, damaged-item notes and a discrepancy callout. |
| 94 | Haiku | A laundry pickup order form with service choices, garment counts, pickup slot and a price summary. |
| 95 | Sonnet | A volunteer shift roster for a food bank with station assignments, vacant slots and a signup form. |
| 96 | Haiku | A music practice log with daily minutes, pieces practiced and a weekly goal progress summary. |
| 97 | Sonnet | A makerspace equipment reservation page with machine availability, safety certification status and a booking form. |
| 98 | Haiku | A plant care guide for three indoor plants with light requirements, watering intervals and seasonal care notes. |
| 99 | Sonnet | A workshop cost estimator with material quantities, unit prices, labor hours and an example total in a summary card. |
| 100 | Haiku | A package return screen with order items, return reason select, pickup address and refund method radio choices. |
| 101 | Sonnet | A language-learning flashcard review summary with words mastered, difficult vocabulary and a suggested next exercise. |
| 102 | Haiku | A wedding seating planner summary with tables, guest names, meal preferences and unassigned guests shown in a separate card. |
| 103 | Sonnet | A bicycle repair intake form with bike model, reported symptoms, service options and an estimate approval checkbox. |
| 104 | Haiku | A museum collection catalog with artifact images, origin, period and curator notes, plus a period filter. |
| 105 | Sonnet | A research participant scheduling page with available sessions, study duration, eligibility checklist and a registration form. |
| 106 | Haiku | A campsite comparison with nightly rates, amenities, maximum tent size and availability for three sites. |
| 107 | Sonnet | A bakery production plan with batches, ingredient requirements, oven slots and packing deadlines in coordinated tables. |
| 108 | Haiku | A home appliance warranty register with purchase dates, coverage expiry and buttons to view receipts. |
| 109 | Sonnet | A scholarship application tracker with eligibility notes, deadlines, required documents and submission-status tags. |
| 110 | Haiku | A chess tournament standings page with players, points, tie-break scores and next-round pairings. |
| 111 | Sonnet | A film festival program with screening rooms, start times, film lengths and tags for sold-out sessions. |
| 112 | Haiku | A household chore rotation with room assignments, responsible person, due day and completion checkboxes. |
| 113 | Sonnet | A nonprofit grant allocation summary with funding by program as a pie chart, beneficiaries served and a restricted-funds note. |
| 114 | Haiku | A board-game lending catalog with player counts, playing time, availability tags and reserve buttons. |
| 115 | Sonnet | A compost monitoring notebook with temperature readings over time, material additions and a moisture observation table. |
| 116 | Haiku | A tailor's fitting appointment card with garment measurements, requested alterations, fittings schedule and estimated price. |
| 117 | Sonnet | A community tool borrowing form with tool select, loan dates, deposit acknowledgment and a short care checklist. |
| 118 | Haiku | A stargazing session planner with celestial targets, viewing windows, equipment checklist and a sky-condition note. |
| 119 | Sonnet | A conference room utilization report with booked hours by room, peak time slots and underused-room recommendations. |
| 120 | Haiku | A pet grooming appointment screen with pet details, service selections, special handling notes and a booking summary. |
| 121 | Sonnet | A pottery studio firing log with kiln loads, clay types, target temperatures, firing stages and inspection results. |
| 122 | Haiku | A moving-day inventory with boxes grouped by room, fragile tags, item counts and a loading checklist. |
| 123 | Sonnet | A student lab equipment checkout page with instrument IDs, calibration dates, current borrower and return deadlines. |
| 124 | Haiku | A neighborhood walking challenge with participant steps, a weekly progress bar chart and milestone badges as tags. |
| 125 | Sonnet | A refill shop shopping list with container tare weights, product price per kilogram and example totals in a table. |
| 126 | Haiku | A genealogy research notebook with ancestor profile cards, document citations and unresolved questions, without a family-tree diagram. |
| 127 | Sonnet | A rowing club training plan with session intensity, distance, crew assignments and a weekly distance chart. |
| 128 | Haiku | A theatre audition schedule with performers, requested roles, audition slots and callback-status tags. |
| 129 | Sonnet | A water-quality fieldwork planner with sampling sites, required bottles, transport assignments and chain-of-custody checklist. |
| 130 | Haiku | A holiday gift exchange organizer with participants, spending limit, wish lists and a checklist of wrapped gifts. |

## Another 20 follow-ups (131–150)

Each changes the named fresh screen in edit mode with its original model.

| # | Model | Of | Request |
|---:|---|---:|---|
| 131 | Sonnet | 51 | Assign the unassigned pitch to Mira, add a fact-check deadline column and mark the first pitch ready for editing. |
| 132 | Haiku | 52 | Move the first unread story to finished and add a topic tag to every saved story. |
| 133 | Sonnet | 55 | Add a fourth candidate with positions on all three issues and highlight the housing row with a short summary callout. |
| 134 | Haiku | 60 | Add a fifth image showing the reopened theatre, with a caption and a restoration completion date. |
| 135 | Sonnet | 61 | Add a library services department and a callout stating which department gained the most funding. |
| 136 | Haiku | 64 | Add a new volunteer request for a river cleanup and group the public meetings by date. |
| 137 | Sonnet | 67 | Add a fourth week's rainfall readings and mark the driest town with a severe-restrictions tag. |
| 138 | Haiku | 70 | Add a fifth novel and place the editor's recommended book first, with a recommendation tag. |
| 139 | Sonnet | 73 | Add a projected enrollment column for next year and flag schools projected to exceed capacity. |
| 140 | Haiku | 80 | Add a new hearing to the procedural timeline and define two more terms in the glossary. |
| 141 | Sonnet | 91 | Record that twelve tomato seed packets were used and add a supplier column to the inventory. |
| 142 | Haiku | 92 | Mark the oldest animal as adopted and add a card for a newly arrived rabbit. |
| 143 | Sonnet | 95 | Fill one vacant packing shift with Sam and add a second sorting shift for Saturday afternoon. |
| 144 | Haiku | 98 | Add a fern with its care requirements and a warning about keeping plants away from a heater. |
| 145 | Sonnet | 103 | Add an optional brake adjustment service and a field for the owner's preferred collection date. |
| 146 | Haiku | 106 | Add a fourth campsite and flag which sites have wheelchair-accessible facilities. |
| 147 | Sonnet | 109 | Mark the earliest scholarship application submitted and add a reminder seven days before each remaining deadline. |
| 148 | Haiku | 112 | Rotate the kitchen and bathroom assignments, and add a monthly window-cleaning chore. |
| 149 | Sonnet | 121 | Mark the first kiln load complete and add an inspection note describing one cracked bowl. |
| 150 | Haiku | 130 | Mark two gifts wrapped and add a delivery deadline and a dietary-preferences note to the organizer. |
