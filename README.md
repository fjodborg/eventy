# Eventy - Discord Verification Bot

> **Note:** This project is still in active development. The name "Eventy" is a remnant from its original purpose as an event management tool - it has since evolved into a verification and server management bot. The name will likely change in the future.

## Background

This bot was developed as a volunteer project to help migrate an organization from Facebook to Discord. Facebook's notification system didn't meet the organization's needs, so I built this bot to handle verification, role-based access control, and seasonal member management.

## Overview

A Discord bot built in Rust that provides automated user verification through OAuth2, role management, and channel access control. The bot supports seasonal member management and ensures only verified users can access member-only channels.

## Features

- OAuth2-based user verification via web interface
- Season-based member and channel management
- Configurable roles and permissions through JSON files
- TLS/HTTPS support for secure verification
- Automatic role assignment and channel permission management

## Installation

### 1. Clone the Repository
```bash
git clone https://github.com/fjodborg/eventy
cd eventy
```

### 2. Environment Setup
Create a `.env` file in the project root:
```env
# Discord Bot Configuration
DISCORD_TOKEN="your_discord_bot_token_here"
DISCORD_CLIENT_ID="your_client_id_here"
DISCORD_CLIENT_SECRET="your_client_secret_here"
DISCORD_GUILD_ID="your_guild_id_here"

# Web Server Configuration
WEB_BASE_URL="https://your-domain.com"
TLS_CERT_PATH=certs/cert.pem
TLS_KEY_PATH=certs/key.pem
HTTPS_PORT=443
HTTP_PORT=80
```

### 3. Discord Bot Setup
1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create a new application
3. Navigate to "Bot" section and copy the bot token
4. Navigate to "OAuth2" section and copy the Client ID and Client Secret
5. Add your redirect URL under OAuth2 > Redirects (e.g., `https://your-domain.com/callback`)
6. Enable the following **Privileged Gateway Intents**:
   - Server Members Intent
   - Message Content Intent

### 4. TLS Certificate Setup
Place your TLS certificates in the `certs/` directory:
```bash
mkdir -p certs
# Add your cert.pem and key.pem files
```

For development/testing, you can use a tunnel service like Cloudflare:
```bash
podman run --rm -it --network host cloudflare/cloudflared:latest tunnel --url http://localhost:3000
```

### 5. Bot Permissions
Invite your bot with the following permissions:
- `Manage Roles`
- `Manage Nicknames`
- `Manage Channels`
- `View Channels`
- `Send Messages`
- `Read Message History`
- `Use Slash Commands`

## Data Structure

The bot uses a JSON-based configuration system:

```
data/
├── global/
│   ├── roles.json        # Role definitions (colors, permissions)
│   ├── permissions.json  # Permission presets (read, readwrite, admin, etc.)
│   └── assignments.json  # User role assignments
└── seasons/
    ├── template/         # Template for new seasons
    │   ├── season.json   # Season configuration
    │   └── users.json    # Member list
    └── 2025E/            # Example season
        ├── season.json
        └── users.json
```

### Roles Configuration (`data/global/roles.json`)
```json
{
  "roles": [
    {
      "name": "Medlem2025E",
      "color": "#2ecc71",
      "hoist": false,
      "mentionable": true,
      "is_default_member_role": true
    }
  ]
}
```

### Permission Presets (`data/global/permissions.json`)
```json
{
  "definitions": {
    "read": {
      "allow": ["VIEW_CHANNEL", "READ_MESSAGE_HISTORY"],
      "deny": ["SEND_MESSAGES"]
    },
    "readwrite": {
      "allow": ["VIEW_CHANNEL", "READ_MESSAGE_HISTORY", "SEND_MESSAGES", "ATTACH_FILES", "ADD_REACTIONS"],
      "deny": []
    }
  }
}
```

### Season Configuration (`data/seasons/<season>/season.json`)
```json
{
  "name": "Spring 2025",
  "active": true,
  "member_role": "Medlem2025E",
  "channels": [
    {
      "name": "general",
      "type": "text",
      "position": 0,
      "role_permissions": {
        "Medlem2025E": "readwrite"
      }
    }
  ]
}
```

### Users Database (`data/seasons/<season>/users.json`)
```json
[
  {
    "Name": "John Doe",
    "DiscordId": "unique-user-id-123"
  }
]
```

## Running the Bot

### Development Mode
```bash
cargo run
```

### Production Mode
```bash
cargo run --release
```

## Usage

### Verification Flow
1. User receives a verification link: `https://your-domain.com/verify/<user-id>`
2. User clicks the link and authenticates with Discord OAuth2
3. Bot verifies the user ID against the database
4. Upon successful verification:
   - User receives the appropriate season member role
   - Nickname is updated to match the database
   - Channel permissions are applied automatically

### Commands
```bash
# Test bot connectivity
/ping

# Manually trigger verification
/verify

# List all users (requires Administrator)
/list_users
```

## Troubleshooting

### Bot Not Responding
- Verify `DISCORD_TOKEN` is correct
- Check bot has required permissions in server
- Ensure Server Members and Message Content intents are enabled

### OAuth Verification Not Working
- Verify `DISCORD_CLIENT_ID` and `DISCORD_CLIENT_SECRET` are correct
- Check `WEB_BASE_URL` matches your redirect URI in Discord Developer Portal
- Ensure TLS certificates are valid and accessible
- Verify the user ID exists in the season's `users.json`

### Permission Errors
- Bot role must be higher than target user roles in the role hierarchy
- Ensure bot has `Manage Roles` and `Manage Nicknames` permissions
- Bot needs `Manage Channels` for channel permission overwrites

## Logging

The bot uses structured logging with levels: ERROR, WARN, INFO, DEBUG

```
2025-01-15T10:30:45.123Z INFO  eventy: Bot logged in as: VerificationBot
2025-01-15T10:30:45.124Z INFO  eventy: Successfully loaded 150 users from database
2025-01-15T10:31:12.456Z INFO  eventy: OAuth verification completed for user: john_doe
```
