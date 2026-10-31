# Project Doc index template

<!-- Keep the applicable variant, replace every placeholder, and use the repository's primary language. -->

## Root index variant

~~~markdown
# {{project_name}} engineering documentation

> This is the Project Doc root index. After loading the project-doc skill, read this entire file on the session's first task in this repository, then follow "Read when" entries to the relevant categories. Reuse previously read content under the skill's "Reading and context reuse" rules. Reread content when it is missing or needs refreshing.

## Documentation scope

{{Describe the durable engineering knowledge covered by the library and the material kept outside it in one or two sentences.}}

## Root-level documents

- [{{document_title}}]({{document_path}}): {{one_sentence_summary}}. Read when: {{read_when}}.

## Categories

- [Architecture](architecture/index.md): {{architecture_summary}}. Read when: {{architecture_read_when}}.
- [Domains](domains/index.md): {{domains_summary}}. Read when: {{domains_read_when}}.
- [Interfaces](interfaces/index.md): {{interfaces_summary}}. Read when: {{interfaces_read_when}}.
- [Operations](operations/index.md): {{operations_summary}}. Read when: {{operations_read_when}}.
- [Decisions](decisions/index.md): {{decisions_summary}}. Read when: {{decisions_read_when}}.

<!-- Keep this section during setup. Show paths as code until their files exist, then add links. -->
## Setup status

- [planned] {{planned_document_path}}: {{planned_scope}}.
- [in_progress] {{current_document_path}}: {{current_scope}}.
~~~

Remove categories without actual documents. Each retained category contains at least one document. When setup is complete, remove "Setup status" and all status prefixes.

## Category index variant

~~~markdown
# {{category_name}} documentation index

> Parent index: [Engineering documentation](../index.md)

## Scope

{{Describe the knowledge owned by this category and the topics routed to other categories.}}

## Documents

- [{{document_title}}]({{document_file}}): {{one_sentence_summary}}. Read when: {{read_when}}.

<!-- During setup, use planned, in_progress, verified, or blocked as needed. Remove statuses after setup. -->
~~~

Give each document one canonical entry in a category index. Use ordinary links between related categories or documents, and keep the summary in the canonical entry.
