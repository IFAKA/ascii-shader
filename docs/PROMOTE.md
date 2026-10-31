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

## Week 1: launch (one place per day, not all at once)
| Day | Where | What to post |
|---|---|---|
| 1 | X and Bluesky | The 10 s MP4 uploaded natively (not a link). Text: the one idea. Repo link in the first reply. |
| 2 | Mastodon (#asciiart #terminal #rust) | Same clip, add alt text describing the scene. |
| 3 | r/commandline, r/asciiart | The clip, title says what it is. Say it is open source, link in a comment. |
| 4 | r/rust, r/unixporn | "How I made a terminal renderer that does not flicker" with the GIF. Read each sub's rules first. |
| 5 | Hacker News "Show HN" | Title: "Show HN: An anime made of terminal text (Rust)". Post in the morning US time. Be present for 3 hours to answer. |
| 7 | Your blog / dev.to | The technical story (below). |

Rules for every post:
- Be plain and honest. Say what is rough (the 3D figures are simple tubes). People trust that.
- Reply to every comment in the first 24 hours. This matters more than the post.
- Never ask for upvotes. Never post the same text twice in one place.
- Credit the ideas you used (the rikiworld devlog). State: no code was copied, ideas only.

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
