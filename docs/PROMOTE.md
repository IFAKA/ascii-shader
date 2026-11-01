# How to promote ascii-shader (build a reputation)

Goal: people remember the name, trust the work, and come back for the next thing.
Reputation comes from many small, honest, good posts over months. One viral post is a bonus, not the plan.

## The one idea to repeat everywhere
"Anime made only of terminal text. The same frames play in your terminal and as an MP4."
Second line, for developers: "Lines and marks instead of brightness ramps, so it does not flicker when the camera moves."
Use these two lines every time, so people learn them.

## Assets (already in the repo)
- `docs/pv-short.mp4` and `docs/pv-short.gif`: 10 s loop with matched sound. Use for social posts.
- `docs/pv.mp4`: the 30 s teaser. Use for YouTube, the README and "watch the full thing" replies.
- Stills: `--film-stills "DIR:t1,t2"` makes PNGs. Post one striking still as the thumbnail.

## Before you post anything (one afternoon)
1. README top: the GIF, the two lines above, one command to try it, a link to the 30 s MP4.
2. Make it easy to run: a release binary on GitHub Releases (macOS and Linux) or `cargo install --git ...`.
   The first thing people do is try it. If that takes more than 2 minutes, they leave.
3. Repo topics: `ascii`, `terminal`, `rust`, `renderer`, `anime`, `shader`.
4. Upload the 30 s MP4 to YouTube (title: "ONE CUT - an anime made of terminal text"). Link the repo in the description.

## Starting point (checked 2026-11-01)
- X @itsfaka: 13 followers, 5 posts. Posts to followers reach almost nobody. Reach comes from replies on other people's posts.
- Reddit: every past post was a link to own project plus "feedback please". Best result: the open-source diffusion course (8 upvotes, 2.3K views, educational). App posts got 0 to 1 upvotes.
- Hacker News: no account. HN limits Show HN from accounts with little history, so build history first.
Sources: [HN karma and Show HN limits](https://news.ycombinator.com/item?id=47718520), [HN account warm-up](https://scout7.ai/academy/launch/how-to-post-a-show-hn-on-hacker-news), [Reddit self-promo rules](https://parsestream.com/guides/reddit-self-promotion-rules), [10% guideline](https://byog.red/guides/reddit-self-promotion-rules/), [reply method on X](https://indieradar.app/blog/reply-guy-method-grow-x-twitter-zero-followers). These are mostly growth blogs; some numbers are unverified. Read each subreddit's rules page yourself.

## Weeks 1 to 4: earn the right to post
Daily (15 to 20 minutes):
- X: reply with real value to 10 to 20 accounts of 5K to 50K followers in ASCII art, creative coding, terminal tools and Rust. Reply early on their posts. When they answer, answer back. Share a fact, a number or a short clip, not "great post".
- HN: make the account today. Leave 1 to 3 careful comments a day on Rust, terminal and graphics threads where you know the topic. Short, honest, no links to your project, no AI-written text.
- Reddit: comment in r/rust, r/commandline, r/asciiart, r/creativecoding. Answer questions. No project links yet.

Posts:
| When | Where | What |
|---|---|---|
| Day 1 | X | The 10 s clip uploaded natively, with the two repeated lines. Repo link in the first reply. |
| Day 3 | Bluesky and Mastodon | Same clip, alt text on. |
| Week 2 | Reddit r/asciiart or r/commandline (after rules check and a few days of comments) | The clip. Title says what it is. Say it is yours and open source. Put the link in a comment. |
| Week 2 or 3 | Reddit r/rust | The flicker story: "a renderer that draws lines and marks, so it does not flicker", with the GIF and the numbers. Technical, not "please give feedback". |
| Week 4 or later | Hacker News | Show HN, only if the account has real comment history. Tuesday to Thursday, 8 to 10 am US Eastern. Stay for 3 hours. |
| Any week | Blog / dev.to | The technical post below. |

Rules for every post:
- Be plain and honest. Say what is rough (tiny faces, simple backdrop). People trust that.
- Reply to every comment in the first 24 hours. This matters more than the post.
- Never ask for upvotes. Never post the same text twice in one place.
- Do not make a second HN or Reddit account if one is restricted.
- Credit the ideas you used (the rikiworld devlog). State: no code was copied, ideas only.
- Do not make every post "please give feedback with a link". Show the work first, link second.

## The technical post (this is what builds trust with developers)
Title: "Why most ASCII renderers flicker, and what I did instead".
Outline:
1. The problem: brightness ramps change characters when the camera moves a little.
2. The idea: draw feature edges as lines and use marks at fixed anchor points. Show a side-by-side GIF (baseline vs yours).
3. The numbers: the metrics the project already prints (changed, flicker, respell cells per frame).
4. The film: the same cell list feeds the terminal player and the MP4 encoder, so both look the same.
5. The sound: every sound comes from the same event list as the picture, so hits land on the exact frame.
6. What failed (the font has no middle dot, tube figures look crude) and what is next.
End with the repo link and one question to the reader. Questions get comments.

## Months 2 to 6: reputation
- Ship one small thing every 2 to 3 weeks and post a 5 to 10 s clip of it: a new scene, a new character, a new effect, a faster mode.
- Keep a changelog and a `docs/devlog/` of short posts. People follow work they can watch grow.
- A second film (a different story, 10 s, same quality) is worth more than ten tweets.
- Make it easy to remix: a short "make your own 10 s film" guide, and an examples folder. Reshare what others make.
- Answer issues fast and be kind. Label "good first issue" so contributors come.
- Be in the places where this audience lives (terminal, creative coding, demoscene, Rust). Comment on others' work before posting your own. Genuine comments, not links.
- Submit to showcases: Rust newsletters (This Week in Rust), creative-coding events, demoscene parties, ASCII art / terminal-art lists.
- Measure: stars per week, clip views, clones. Repeat what worked, drop what did not.

## What to avoid
- Posting everywhere on day 1: you cannot answer everyone and each place sees a weak launch.
- Claims you cannot show ("fastest", "best"). Show a clip or a number.
- Silence after the post. The first 24 hours decide how far it travels.
