/**
 * WhatsApp client wrapper using Baileys.
 *
 * Manages connection to WhatsApp via the multi-device protocol,
 * handles QR authentication, message extraction, and auto-reconnect.
 */

/* eslint-disable @typescript-eslint/no-explicit-any */
import makeWASocket, {
  DisconnectReason,
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
  makeCacheableSignalKeyStore,
} from '@whiskeysockets/baileys';

import { Boom } from '@hapi/boom';
import qrcode from 'qrcode-terminal';
import pino from 'pino';

const VERSION = '0.1.0';

// ─────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────

/** Message received from WhatsApp, forwarded to the Rust bot. */
export interface InboundMessage {
  id: string;
  sender: string;
  pn: string;
  content: string;
  timestamp: number;
  isGroup: boolean;
}

/** Options for creating the WhatsApp client. */
export interface WhatsAppClientOptions {
  /** Directory to store auth credentials. */
  authDir: string;
  /** Callback when a message is received. */
  onMessage: (msg: InboundMessage) => void;
  /** Callback when a QR code is generated. */
  onQR: (qr: string) => void;
  /** Callback for connection status changes. */
  onStatus: (status: string) => void;
}

// ─────────────────────────────────────────────
// WhatsAppClient
// ─────────────────────────────────────────────

export class WhatsAppClient {
  private sock: any = null;
  private options: WhatsAppClientOptions;
  private reconnecting = false;

  constructor(options: WhatsAppClientOptions) {
    this.options = options;
  }

  /** Connect to WhatsApp using Baileys. */
  async connect(): Promise<void> {
    const logger = pino({ level: 'silent' });
    const { state, saveCreds } = await useMultiFileAuthState(
      this.options.authDir
    );
    const { version } = await fetchLatestBaileysVersion();

    console.log(`[bridge] using Baileys v${version.join('.')}`);

    this.sock = makeWASocket({
      auth: {
        creds: state.creds,
        keys: makeCacheableSignalKeyStore(state.keys, logger),
      },
      version,
      logger,
      printQRInTerminal: false,
      browser: ['oxibot', 'cli', VERSION],
      syncFullHistory: false,
      markOnlineOnConnect: false,
    });

    // Handle WebSocket transport errors
    if (this.sock.ws && typeof this.sock.ws.on === 'function') {
      this.sock.ws.on('error', (err: Error) => {
        console.error('[bridge] ws error:', err.message);
      });
    }

    // ── Connection lifecycle ──
    this.sock.ev.on('connection.update', async (update: any) => {
      const { connection, lastDisconnect, qr } = update;

      if (qr) {
        console.log(
          '\n📱 Scan this QR code with WhatsApp → Linked Devices:\n'
        );
        qrcode.generate(qr, { small: true });
        this.options.onQR(qr);
      }

      if (connection === 'close') {
        const statusCode = (lastDisconnect?.error as Boom)?.output?.statusCode;
        const shouldReconnect = statusCode !== DisconnectReason.loggedOut;

        console.log(
          `[bridge] connection closed (status=${statusCode}, reconnect=${shouldReconnect})`
        );
        this.options.onStatus('disconnected');

        if (shouldReconnect && !this.reconnecting) {
          this.reconnecting = true;
          console.log('[bridge] reconnecting in 5 s …');
          setTimeout(() => {
            this.reconnecting = false;
            this.connect();
          }, 5000);
        }
      } else if (connection === 'open') {
        console.log('[bridge] ✅ connected to WhatsApp');
        this.options.onStatus('connected');
      }
    });

    this.sock.ev.on('creds.update', saveCreds);

    // ── Incoming messages ──
    this.sock.ev.on(
      'messages.upsert',
      async ({ messages, type }: { messages: any[]; type: string }) => {
        if (type !== 'notify') return;

        for (const msg of messages) {
          // Skip own messages and status broadcasts
          if (msg.key.fromMe) continue;
          if (msg.key.remoteJid === 'status@broadcast') continue;

          const content = this.extractMessageContent(msg);
          if (!content) continue;

          const isGroup =
            msg.key.remoteJid?.endsWith('@g.us') || false;

          this.options.onMessage({
            id: msg.key.id || '',
            sender: msg.key.remoteJid || '',
            pn: msg.key.remoteJidAlt || '',
            content,
            timestamp: msg.messageTimestamp as number,
            isGroup,
          });
        }
      }
    );
  }

  // ── Content extraction ──

  /**
   * Extract the human-readable text from a Baileys message object.
   * Handles plain text, extended text (links/replies), media captions,
   * and voice messages.
   */
  private extractMessageContent(msg: any): string | null {
    const message = msg.message;
    if (!message) return null;

    // Plain text
    if (message.conversation) return message.conversation;

    // Extended text (reply, link preview, etc.)
    if (message.extendedTextMessage?.text)
      return message.extendedTextMessage.text;

    // Media with captions
    if (message.imageMessage?.caption)
      return `[Image] ${message.imageMessage.caption}`;
    if (message.videoMessage?.caption)
      return `[Video] ${message.videoMessage.caption}`;
    if (message.documentMessage?.caption)
      return `[Document] ${message.documentMessage.caption}`;

    // Voice / audio (no caption — just a placeholder)
    if (message.audioMessage) return '[Voice Message]';

    return null;
  }

  /** Send a text message to a WhatsApp JID. */
  async sendMessage(to: string, text: string): Promise<void> {
    if (!this.sock) throw new Error('Not connected');
    await this.sock.sendMessage(to, { text });
  }

  /** Gracefully disconnect from WhatsApp. */
  async disconnect(): Promise<void> {
    if (this.sock) {
      this.sock.end(undefined);
      this.sock = null;
    }
  }
}
