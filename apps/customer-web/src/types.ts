import type { Schema } from "@piggybank/web-core";
export type Application = Schema["Application"];
export type Product = Schema["ProductVersion"];
export type Account = Schema["AccountSummary"];
export type Cases = {
  id: string;
  state: string;
  createdAt: string;
  message: string;
}[];
