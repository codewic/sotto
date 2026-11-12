# Developer features

Status: built and tested (12 November 2026).

## Code in messages

Wrap code in three backticks, with an optional language: ` ```ts `, ` ```py `, ` ```sh `, ` ```sql `, ` ```rust `, ` ```go `, ` ```json `, ` ```yaml `, ` ```css `, and the C family for anything else. Code shows in a monospace block with highlighting, a language label and a Copy button, in 1:1 conversations, groups and channel posts. Inline `code` works as before.

The highlighter is a small tokenizer in `apps/desktop/src/ui/highlight.ts`. It only splits text into tokens and never builds HTML, so a message can't become markup or script. List previews drop the fences and keep the code on one line.

## Integrations: posts from scripts, CI and monitoring

Settings, Integrations: make an integration, name it (for example CI alerts), and choose a group, or a channel you made, for it to post to. You get an address and a token once; Sotto keeps only the token's hash.

- **Where it listens.** Only on this computer: the app's private socket (`unix:/path/agent.sock/hooks/<id>`, readable only by your user account), or `http://127.0.0.1:<port>/hooks/<id>` in development. It's a separate route from the app's own API, with its own token: the app's token doesn't work there, and an integration's token doesn't open the app.
- **What it posts.** Plain text, or JSON `{"text": "..."}`, up to 4,000 characters, 30 posts a minute per integration. Posts are encrypted and sent like any message. In a group they're labelled "<integration> (via <you>)" so nobody takes a script for a person.
- **When it works.** While Sotto is open and unlocked. A locked app answers 503 and posts nothing.

### `sotto-hook`

A small command with no dependencies (`crates/sotto-hook`, build with `cargo build --release -p sotto-hook`):

```sh
export SOTTO_HOOK_URL=unix:/home/you/.config/Sotto/agent.sock/hooks/<id>
export SOTTO_HOOK_TOKEN=sotto_hook_...

sotto-hook "Deploy 412 finished"
make test 2>&1 | tail -20 | sotto-hook --code sh
```

It reads the message from its arguments or from standard input, and exits non-zero with Sotto's reason if the post is refused. Pass the token in the environment rather than `--token`, which other users on the computer can see in the process list. With curl: `curl --unix-socket <socket> -H "Authorization: Bearer $SOTTO_HOOK_TOKEN" --data "text" http://localhost/hooks/<id>`.

## One-time secrets

In a 1:1 conversation, the key button sends a one-time secret: a password, an API key, a recovery code.

- It travels end-to-end encrypted like any message, as its own payload. The sender's device keeps only its label.
- The receiver sees the label and "Reveal once". Revealing shows the text a single time and deletes it from their device in the same step; the sender's card then says when it was opened.
- If nobody opens it, it's destroyed after 1 hour, 1 day or 7 days (sender's choice, never more than 7 days, enforced on the receiving side too).
- The reveal dialog offers Copy and clears the clipboard after a minute.
- Secret text never appears in a listing, an event, a log (the payload's `Debug` output hides it) or a preview.

Limits: the receiver can still copy or photograph a secret once it's shown, and a compromised receiving device can read it before it's opened. One-time means it can't be revealed again from Sotto, not that it can't be kept.

## Tests

- `crates/sotto-agent/tests/agent.rs` `integrations_post_and_secrets_open_once`: a group integration posts with its label; a wrong token, a non-local host and the app's own token are refused; the hook token can't call the app's API; a removed integration stops working; a secret opens once, never shows in listings, the sender can't open it, and the sender sees "opened".
- `crates/sotto-hook`: address parsing (socket and TCP, local only), error messages, arguments.
- `apps/desktop/src/ui/highlight.test.ts` (`bun test`): fences, tokens per language, previews.
- Live: `sotto-hook` posted over TCP to the dev desks and over a Unix socket to a throwaway agent; code blocks, the integration dialog and a secret's send, reveal and "opened" state checked in the app.
