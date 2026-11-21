# Changes

## 0.1.18

- Warn about the account risk before every sign-in
- Add "Free models only" for each site, on by default: list only models free now and refuse the others before sending. Update a saved channel's models once to keep using it
- Turn automatic check-in off by default; saved settings keep their choice
- Follow the current WorkBuddy client identity and recognize two more check-in answers

## 0.1.17

- Pause hidden credit refresh timers and reuse results until the refresh interval expires

## 0.1.16

- Preserve tool declarations when disabling tool calls for context summaries

## 0.1.15

- Read provider accounts and new contexts concurrently

## 0.1.14

- Preserve structured upstream error diagnostics for model recovery.

## 0.1.13

- Use the WorkBuddy logo for the package and its channel types

## 0.1.12

- Show shared account credits in channel rows
- Configure automatic daily check-in with a time control and enable switch

## 0.1.11

- Share credit reads across sessions using the current account credential
- Align credit query ranges with the existing refresh interval

## 0.1.10

- Keep credit queries silent and hide unavailable balances

## 0.1.9

- Add localized catalog descriptions

## 0.1.8

- Register China daily credit claims with the execution Node scheduler
- Configure the daily check-in time in Beijing time, including when another model channel is selected
- Keep closed activities, completed claims and uncertain requests from repeating

## 0.1.7

- Remove Hy4's trailing reasoning control token after normal stream completion
- Show a credit icon beside the balance

## 0.1.6

- Send the declared regional gateway identity for chat and credit requests
- Use the CodeBuddy International CLI identity and request streamed usage

## 0.1.5

- Show zero credits when the upstream resource list is explicitly null
- Preserve streamed usage before completing a model response

## 0.1.4

- Send the desktop generation identity and preserve upstream business messages
- Show selected-channel credits through protected read-only requests

## 0.1.3

- Send regional discovery headers and accept wrapped or bare model catalogs
- Group regional channels under WorkBuddy

## 0.1.2

- Add marketplace category and icon color
