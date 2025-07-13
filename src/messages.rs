// src/messages.rs

pub fn welcome_message(name: &str) -> String{
    
    format!(
        "Welcome **{}**!\n\n\
        🔐 **Verification Required**\n\
        To access all server channels, you need to verify your identity.\n\n\
        📨 **Check Your Private Messages**\n\
        I've sent you a private message with verification instructions.\n\
        Please check your DMs and follow the instructions there.\n\n\
        ❓ **Need Help?**\n\
        If you don't receive a DM or need assistance, please contact an administrator.\n\n\
        This message is only visible to you.",
        name
    )
}        
pub fn verification_message(name: &str) -> String{
    
    format!(
        "👋 **Hello, {}!**\n\n\
        🔐 **Identity Verification Required**\n\n\
        To gain full access to the server, you need to verify your identity.\n\
        Please provide your user ID by replying to this message.\n\n\
        **Simply reply with your user ID.**\n\n\
        Example: `5342a99-5a43-112g-d771-s34233v38g11`\n\n\
        If you don't know your user ID or need help, please contact an administrator in the server.\n\n\
        Thank you for your cooperation! 🙂",
        name
    )
}            

pub fn success_message(name: &str) -> String{
    format!(
        "✅ **Verification Successful!**\n\n\
        Welcome, **{}**!\n\n\
        Your identity has been verified and I'm now updating your server access:\n\
        • Setting your nickname to: **{}**\n\
        • Assigning you the Member role\n\
        • Granting access to member channels\n\n\
        You should now have full access to the server. If you encounter any issues, please contact an administrator.",
        name, name
    )
}
pub fn error_message(name: &str) -> String{
    format!(
        "❌ **User ID Not Found**\n\n\
        The user ID `{}` was not found in our database.\n\n\
        Please double-check your user ID and try again, or contact an administrator if you believe this is an error.\n\n\
        **Simply reply with your correct user ID to try again.**",
        name
    )
}