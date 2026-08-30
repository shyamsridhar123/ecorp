# Evidence verification validation — August 29, 2026

## Scope

This record covers issue #14: runner-side automated checks, linked evidence, blocked completion,
human approval, and independent review.

## Automated matrix

Mission `74ada636-00e4-4f10-b165-a36952e83b0f` produced run
`ca020433-fb08-414b-9a82-f81cbe0689f1`.

Six linked evidence records passed in policy order:

1. artifact bytes and SHA-256
2. required file
3. direct command
4. test command
5. JSON required-key schema
6. PNG screenshot signature

The run and task reached `verification_status=passed` before the run completed.

## Failed verification

Mission `b91d333c-2c77-42a5-bcc1-464c9f4944c8` intentionally omitted
`missing-required.txt`.

- artifact check: passed
- required-file check: failed
- task state: `verification_failed`
- run state: failed
- mission state: failed
- accepted `run.completed` events: zero

## Human and independent gates

- Human-approval run `b93a1a31-95e0-489f-ac99-f8c1bc6d8fed` waited after automated evidence,
  then completed after Alice's owner-role approval; a repeated decision returned HTTP 400.
- Independent-review run `a0f8363b-18d5-478e-99f1-4210af26e539` rejected Alice with HTTP 403
  because she requested the mission, then completed after Bob's reviewer decision.

## Unit coverage

Runner tests verify successful matrix execution plus missing-file and path-traversal failures.
Server policy validation rejects unsafe paths, invalid commands, empty gates, and malformed check
limits. The server also rejects premature completion, incomplete passing evidence, mismatched
manual gates, non-human decisions, self-review, and repeated decisions.
