# Discord Verification Bot
A Discord bot built in Rust that provides automated user verification through private messages, role management, and channel access control. The bot maintains a user database and ensures only verified users can access member-only channels.

## 🛠️ Installation

### 1. Clone the Repository
```bash
git clone https://github.com/fjodborg/eventy
cd discord-verification-bot
```

### 2. Environment Setup
Modify the `.env` file in the project root:
```env
DISCORD_TOKEN=your_discord_bot_token_here
```

### 3. Database Setup
Modify the `data/users.json` with your data. Example:
``` json
[ 
  {
    \"Name\": \"John Doe\",
    \"DiscordId\": \"user123456\"
  },
  {
    \"Name\": \"Jane Smith\", 
    \"DiscordId\": \"user789012\"
  }
]
```

### 4. Discord Bot Setup
1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create a new application
3. Navigate to \"Bot\" section
4. Copy the bot token to your `.env` file
5. Enable the following **Privileged Gateway Intents**:
   - Server Members Intent
   - Message Content Intent

### 5. Bot Permissions
Invite your bot with the following permissions:
- `Manage Roles`
- `Manage Nicknames`
- `Manage Channels`
- `View Channels`
- `Send Messages`
- `Read Message History`
- `Use Slash Commands`


## 🏃‍♂️ Running the Bot

### Development Mode
```bash
cargo run
```

### Production Mode
```bash
cargo run --release
```

### Using Docker (Optional)
TODO: Implement docker solution.
## 📚 Usage

### For New Members
The flow looks like this:
1. User gets a welcome messages to check DM.
2. In the DM the user is requested an userid.
3. Once the ID has been validated the user gets a define name assigned and the corresponding channel permissions. 

### Commands

``` bash
#Test bot connectivity and responsiveness.
/ping
# Manually start the verification process for yourself.
/verify
# Display all users in the database (requires Administrator permission).
/list_users
```

### Verification Flow Example
```
Bot: 👋 Hello, John!

🔐 Identity Verification Required

To gain full access to the server, you need to verify your identity.
Please provide your user ID by replying to this message.

Simply reply with your user ID.

Example: SomeId

User: user123456

Bot: ✅ Verification Successful!

Welcome, John Doe!

Your identity has been verified and I'm now updating your server access:
• Setting your nickname to: John Doe
• Assigning you the Member role
• Granting access to member channels
```

## 🔧 Configuration

### Channel Access Rules
TODO: This section needed rework, once things aren't hardcoded.
The bot automatically manages channel access based on naming conventions:

**Always Accessible** (Unverified + Verified):
- `welcome`
- `rules` 
- `announcements`
- `verification`

**Member Only** (Verified):
- `general`
- `chat`
- `discussion`
- `off-topic`
- All other channels (default)

### Role Management
- **Unverified Role**: Automatically removed upon verification
- **Member Role**: Automatically assigned upon verification

*Note: These roles must exist in your Discord server for the bot to work*

### Customization
TODO: This section needed rework, once things aren't hardcoded.
Modify message templates in `src/messages.rs`:
- `welcome_message()`: Server welcome message
- `verification_message()`: DM verification prompt
- `success_message()`: Verification success confirmation
- `error_message()`: Verification failure notification


The bot uses structured logging with different levels:

- **ERROR**: Errors requiring attention
- **WARN**: Issues that should be monitored
- **INFO**: General operational information
- **DEBUG**: Detailed debugging information

### Log Output Example
TODO: Add some sort of log output for admins.
```
2024-01-15T10:30:45.123Z INFO  discord_verification_bot: Bot logged in as: VerificationBot
2024-01-15T10:30:45.124Z INFO  discord_verification_bot: Successfully loaded 150 users from database
2024-01-15T10:31:12.456Z INFO  discord_verification_bot::user_manager: New member joined: John Doe (123456789) in guild: 987654321
2024-01-15T10:31:12.789Z DEBUG discord_verification_bot::commands: Starting DM verification process for: John Doe
```

## 🚨 Troubleshooting

### Common Issues

#### Bot Not Responding
- **Check Token**: Ensure `DISCORD_TOKEN` is correct
- **Verify Permissions**: Bot needs required permissions in server
- **Check Intents**: Enable Server Members and Message Content intents

#### Verification Not Working
- **Database Format**: Ensure `users.json` follows correct structure
- **User ID Match**: Verify user IDs in database match expected format
- **DM Permissions**: Users must allow DMs from server members

#### Permission Errors
- **Role Hierarchy**: Bot role must be higher than target user roles
- **Missing Permissions**: Ensure bot has `Manage Roles` and `Manage Nicknames`
- **Channel Permissions**: Bot needs `Manage Channels` for permission overwrites

### Health Checks
Use the `/ping` command to verify bot connectivity and responsiveness.

### TODO Roadmap
- [ ] Add proper Database access instead of json.
- [ ] Add option to `/verify` through DM.
- [ ] Add event planning.
- [ ] Add log output for admin users.
- [ ] Add configuration messages to setup welcome messages, validation formats, roles, users access etc.
- [ ] Create file/command for configuring channels that should exist and under which category.