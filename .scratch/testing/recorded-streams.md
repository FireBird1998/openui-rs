# Fifty recorded streams: the requests

Testing ticket 11. Record each in the viewer (Ask Claude), in the **react-ui** library (the default), at **low** effort, with the model shown. Follow-ups (41–50) are asked from the screen of the fresh request they name, with **Ask for a change** (edit mode). Then run `scripts/import-recordings.sh`, which takes the recordings from the viewer's folder.

**Estimated spend:** about $1.10 in all, at the viewer's prices and its prompt (about 9,500 tokens): roughly 4 cents per Sonnet 5.5 request and 0.2 cents per Haiku 5.5 one, with outputs of 1,500 to 2,000 tokens. Well inside the $10 cap, which the viewer enforces.

## Fresh screens (40)

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

## Follow-ups (10)

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
