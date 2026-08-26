CREATE TABLE `messages` (
	`id` text PRIMARY KEY NOT NULL,
	`session_id` text NOT NULL,
	`turn_id` text,
	`position` integer NOT NULL,
	`role` text NOT NULL,
	`content` text NOT NULL,
	`created_at` integer NOT NULL,
	`streaming` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `messages_by_session` ON `messages` (`session_id`,`position`);--> statement-breakpoint
CREATE TABLE `projects` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`path` text NOT NULL,
	`position` integer NOT NULL,
	`created_at` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `session_details` (
	`session_id` text PRIMARY KEY NOT NULL,
	`data` text NOT NULL
);
--> statement-breakpoint
CREATE TABLE `sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`project_id` text NOT NULL,
	`title` text NOT NULL,
	`provider` text NOT NULL,
	`model` text,
	`status` text NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	`last_reply_at` integer
);
--> statement-breakpoint
CREATE INDEX `sessions_by_project` ON `sessions` (`project_id`,`updated_at`);--> statement-breakpoint
CREATE INDEX `sessions_by_updated_at` ON `sessions` (`updated_at`);--> statement-breakpoint
CREATE INDEX `sessions_by_last_reply_at` ON `sessions` (`last_reply_at`);