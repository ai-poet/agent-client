ALTER TABLE `messages` ADD `display_content` text;--> statement-breakpoint
ALTER TABLE `messages` ADD `attachments` text DEFAULT '[]' NOT NULL;