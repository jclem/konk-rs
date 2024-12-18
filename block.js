// Save as test-signal.js and run with: node test-signal.js
// Press Ctrl+C (SIGINT) or send SIGTERM (kill <pid>) to test.
// On receiving SIGINT or SIGTERM, it will wait 5 seconds before exiting with code 0.

process.on("SIGINT", () => {
  console.log("Received SIGINT, will exit in 5s...");
  setTimeout(() => process.exit(0), 5000);
});

process.on("SIGTERM", () => {
  console.log("Received SIGTERM, will exit in 5s...");
  setTimeout(() => process.exit(0), 5000);
});

// Keep the process alive indefinitely so we can send signals
setInterval(() => {
  console.log("ping");
}, 1000);
