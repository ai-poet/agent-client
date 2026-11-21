# WorkBuddy

An optional Sailry model provider package. Sign-in, refresh, HTTP, cancellation
and credentials stay on the execution Node. The translation module receives
conversation content, but not credentials. A separate, explicitly consented
authentication transform normalizes token response fields.

The package offers China, WorkBuddy International and CodeBuddy International
routes. Select the route for the account being signed in. Model discovery reads
that route's real configuration catalog; it does not infer capabilities from
model names or invent fallback model limits.

The package does not read another application's profile, import its credentials,
manage an account pool, inspect billing, or replace the Agent engine. Sign-in
uses the service's browser challenge. Account-profile metadata is not fetched;
requests use the upstream's supported missing-user and missing-enterprise headers.

Protocol rules come from the reviewed third-party revision in NOTICE. Synthetic
package fixtures cover wire behavior; they do not prove live account access.
There is no automatic account switching, request replay or DSML tool recovery.

## Account risk

Tencent may suspend an account used outside its own apps, and accounts of the
international build have been banned for it. Every sign-in shows this warning
once and starts only after it is confirmed.

## Free models only

Each site has a "Free models only" setting, on by default. This default is a
deliberate difference from the reference in NOTICE, where it starts off.

- A model is free when the site's catalog prices it at zero credits, or when a
  running whole-day promotion does. A promotion limited to hours of the day and
  a model without a stated price both count as charged.
- "Update models" lists only the models free at that moment and records the
  price of each one with the channel.
- A request for a model that is charged, has no price record or has an outdated
  one is refused by the package before anything is sent, with a message that
  says why.
- With the setting off, every model is listed and sent. Models hidden earlier
  return after the next "Update models".

Prices are read only when the models are updated; Sailry does not refresh them
on a timer. A model that stops being free in between is still sent until its
promotion's stated end, until its record is seven days old, or until the models
are updated, whichever comes first. Update the models to check sooner. A
channel saved before this version has no price records, so it sends nothing
until its models are updated or the setting is turned off.

## Daily check-in

Automatic check-in is off by default and is turned on in the package settings.
An installation whose settings were saved before keeps its saved choice; one
that never saved settings follows the new default and stops claiming until it
is turned on. The manual check-in action is unaffected.
