# oxibot WhatsApp Bridge

Node.js sidecar process that connects to WhatsApp via the [Baileys](https://github.com/WhiskeySockets/Baileys) library and exposes a WebSocket server for the Rust oxibot process.

## Architecture

```
┌────────────────────────┐  WebSocket (ws://localhost:3001)  ┌──────────────────┐
│  Node.js Bridge        │◄────────────────────────────────►│  oxibot (Rust)   │
│  (TypeScript + Baileys)│                                   │  WhatsAppChannel │
└────────────────────────┘                                   └──────────────────┘
```

The bridge handles the WhatsApp Web protocol (multi-device) and translates messages into simple JSON frames over WebSocket.

## Protocol

### Bridge → oxibot (inbound)

```jsonc
// Incoming WhatsApp message
{ "type": "message", "id": "ABC", "sender": "1234@lid", "pn": "1234@s.whatsapp.net", "content": "Hello", "timestamp": 1700000000, "isGroup": false }

// QR code for authentication
{ "type": "qr", "qr": "2@BASE64..." }

// Connection status change
{ "type": "status", "status": "connected" | "disconnected" }

// Error
{ "type": "error", "error": "description" }
```

### oxibot → Bridge (outbound)

```jsonc
// Send a text message
{ "type": "send", "to": "1234@lid", "text": "Reply text" }
```

### Bridge → oxibot (ack)

```jsonc
{ "type": "sent", "to": "1234@lid" }
{ "type": "error", "error": "Not connected" }
```

## Quick Start

```bash
# Install dependencies
npm install

# Build TypeScript
npm run build

# Run the bridge
npm start
```

## Environment Variables

| Variable      | Default                          | Description                       |
|---------------|----------------------------------|-----------------------------------|
| `BRIDGE_PORT` | `3001`                           | WebSocket server port             |
| `AUTH_DIR`    | `~/.oxibot/whatsapp-auth`       | WhatsApp auth state directory     |

## First-time Setup

1. Start the bridge: `npm start`
2. A QR code will appear in the terminal
3. Open WhatsApp on your phone → Settings → Linked Devices → Link a Device
4. Scan the QR code
5. Auth state is saved to `AUTH_DIR` — subsequent starts are automatic

## Docker

When using the oxibot Docker image, the bridge is built and bundled automatically. Start it alongside oxibot using the provided entrypoint or docker-compose.
