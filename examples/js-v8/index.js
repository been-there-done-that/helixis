console.log("\n--- [Helixis V8 Node Sandbox] ---");
console.log("Processing generic payload via V8 runtime engines...");
const payload = {
    event: "user_login",
    timestamp: new Date().toISOString(),
    status: "Processed"
};
console.log(`Node Output: ${JSON.stringify(payload)}`);
console.log("---------------------------------\n");
