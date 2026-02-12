/**
 * WebSocket server for oxibot ↔ bridge communication.
 *
 * The server listens on a configurable port (default 3001).
 * The Rust oxibot process connects as a WebSocket **client** and exchanges
 * JSON messages following this protocol:
 *
 * Bridge → Rust:
 *   {"type":"message","id":"...","sender":"...","pn":"...","content":"...","timestamp":N,"isGroup":bool}
 *   {"type":"qr","qr":"..."}
 *   {"type":"status","status":"connected"|"disconnected"}
 *   {"type":"error","error":"..."}
 *
 * Rust → Bridge:
 *   {"type":"send","to":"...","text":"..."}
 *
 * Bridge → Rust (ack):
 *   {"type":"sent","to":"..."}
 *   {"type":"error","error":"..."}
 */

import { WebSocketServer, WebSocket } from 'ws';
import { WhatsAppClient, InboundMessage } from './whatsapp.js';

// ─────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────

/** Outbound command from the Rust bot. */
interface SendCommand {
  type: 'send';
  to: string;
  text: string;
}

/** Events broadcast from the bridge to the Rust bot. */
interface BridgeEvent {
  type: 'message' | 'status' | 'qr' | 'error';
  [key: string]: unknown;
}

// ─────────────────────────────────────────────
// BridgeServer
// ─────────────────────────────────────────────

export class BridgeServer {
  private wss: WebSocketServer | null = null;
  private wa: WhatsAppClient | null = null;
  private clients: Set<WebSocket> = new Set();

  constructor(
    private port: number,
    private authDir: string
  ) {}

  /** Start the WebSocket server and connect to WhatsApp. */
  async start(): Promise<void> {
    this.wss = new WebSocketServer({ port: this.port });
    console.log(
      `[bridge] 🌉 server listening on ws://localhost:${this.port}`
    );

    this.wa = new WhatsAppClient({
      authDir: this.authDir,
      onMessage: (msg: InboundMessage) =>
        this.broadcast({ type: 'message', ...msg }),
      onQR: (qr: string) => this.broadcast({ type: 'qr', qr }),
      onStatus: (status: string) =>
        this.broadcast({ type: 'status', status }),
    });

    this.wss.on('connection', (ws: WebSocket) => {
      console.log('[bridge] 🔗 oxibot client connected');
      this.clients.add(ws);

      ws.on('message', async (data: Buffer | string) => {
        try {
          const cmd = JSON.parse(data.toString()) as SendCommand;
          await this.handleCommand(cmd, ws);
        } catch (error) {
          console.error('[bridge] error handling command:', error);
          ws.send(JSON.stringify({ type: 'error', error: String(error) }));
        }
      });

      ws.on('close', () => {
        console.log('[bridge] 🔌 oxibot client disconnected');
        this.clients.delete(ws);
      });

      ws.on('error', (error: Error) => {
        console.error('[bridge] client ws error:', error.message);
        this.clients.delete(ws);
      });
    });

    await this.wa.connect();
  }

  /** Handle an outbound command from oxibot. */
  private async handleCommand(
    cmd: SendCommand,
    ws: WebSocket
  ): Promise<void> {
    if (cmd.type !== 'send') {
      ws.send(
        JSON.stringify({
          type: 'error',
          error: `unknown command type: ${cmd.type}`,
        })
      );
      return;
    }

    if (!this.wa) {
      ws.send(
        JSON.stringify({ type: 'error', error: 'WhatsApp not connected' })
      );
      return;
    }

    await this.wa.sendMessage(cmd.to, cmd.text);
    ws.send(JSON.stringify({ type: 'sent', to: cmd.to }));
  }

  /** Broadcast a bridge event to all connected oxibot clients. */
  private broadcast(event: BridgeEvent): void {
    const data = JSON.stringify(event);
    for (const client of this.clients) {
      if (client.readyState === WebSocket.OPEN) {
        client.send(data);
      }
    }
  }

  /** Gracefully shut down the bridge. */
  async stop(): Promise<void> {
    for (const client of this.clients) {
      client.close();
    }
    this.clients.clear();

    if (this.wss) {
      this.wss.close();
      this.wss = null;
    }

    if (this.wa) {
      await this.wa.disconnect();
      this.wa = null;
    }
  }
}
