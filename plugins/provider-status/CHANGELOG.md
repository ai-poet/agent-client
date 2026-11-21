# Changelog

## 0.1.10

- Use a new plugin icon

## 0.1.9

- Resume refresh requests deferred by account cooldowns without periodic polling
- Coalesce deferred reads and discard them when their view is hidden or account changes

## 0.1.8

- Refresh account usage on visible activity and completed work instead of periodic timers
- Preserve account results and in-flight reads across unchanged model contexts

## 0.1.7

- Pause account refresh timers while their views are hidden and reuse unexpired results on reveal

## 0.1.6

- Read provider accounts and new contexts concurrently

## 0.1.5

- Label 5-hour and 7-day usage windows as 5h and 7d

## 0.1.4

- Show remaining usage and shared account measurements in channel rows
- Hide unavailable percentages and keep queries silent

## 0.1.3

- Show reset times in the device local time zone with a concise shared label
- Share usage reads across sessions using the current account credential

## 0.1.2

- Keep account usage reads silent and hide unavailable measurements
- Preserve zero for successful nullable measurements

## 0.1.1

- Fix package metadata and localized catalog descriptions

## 0.1.0

- Read configured GPT and Claude OAuth account usage in the shared footer
