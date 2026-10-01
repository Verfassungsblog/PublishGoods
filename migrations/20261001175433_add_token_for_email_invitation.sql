-- Add migration script here
alter table invitations
    add email_token text;