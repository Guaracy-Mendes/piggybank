import createClient from 'openapi-fetch';
import type { paths } from './schema';

/** Browser credentials stay in the BFF's opaque session; no token storage here. */
export function createBankClient(baseUrl = '') {
  return createClient<paths>({ baseUrl, credentials: 'include' });
}

export type { components, paths } from './schema';
