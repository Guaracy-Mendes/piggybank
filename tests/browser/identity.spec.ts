import { test, expect } from '@playwright/test';

test('real browser uses issuer PKCE login and keeps customer and workforce sessions separate', async ({ page, context }) => {
  await page.goto('/auth/customer/login');
  await page.locator('#username').fill('alice');
  await page.locator('#password').fill('Synthetic-Local-123!');
  await page.locator('#kc-login').click();
  await page.waitForURL('**/auth/customer/me');
  const identity = JSON.parse(await page.locator('body').innerText());
  expect(identity.actor.trust).toBe('CUSTOMER');
  expect(identity.actor.clientId).toBe('customer-web');
  expect(Object.keys(identity)).toEqual(['actor', 'expiresAt']);
  const cookies = await context.cookies();
  expect(cookies.find(cookie => cookie.name === 'pb_customer')?.httpOnly).toBe(true);
  const staff = await context.request.get('/auth/workforce/me');
  expect(staff.status()).toBe(401);
  const missingCsrf = await context.request.post('/auth/customer/logout');
  expect(missingCsrf.status()).toBe(403);
  const csrf = cookies.find(cookie => cookie.name === 'pb_csrf_customer')?.value;
  expect(typeof csrf).toBe('string');
  const logout = await context.request.post('/auth/customer/logout', {
    headers: { 'X-CSRF-Token': csrf!, Origin: 'http://127.0.0.1:4180' },
    maxRedirects: 0,
  });
  expect(logout.status()).toBe(303);
  const revoked = await context.request.get('/auth/customer/me');
  expect(revoked.status()).toBe(401);
});
