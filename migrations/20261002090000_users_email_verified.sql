-- Whether the account owner has proven control of the email address. Only verified emails are
-- matched against pending email invitations. Existing accounts are treated as verified.
ALTER TABLE users ADD COLUMN email_verified boolean NOT NULL DEFAULT true;
