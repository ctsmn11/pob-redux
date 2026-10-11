import { gggAccount, gggSignIn, gggSignInCancel, gggSignOut, type GggAccount } from "$lib/engine.svelte";

const DAY_MS = 86_400_000;

class GggStore {
  account = $state<GggAccount | null>(null);
  signingIn = $state(false);

  get daysLeft(): number {
    return this.account ? Math.ceil((this.account.endsAt * 1000 - Date.now()) / DAY_MS) : 0;
  }

  async sync() {
    this.account = await gggAccount().catch(() => null);
  }

  /** Resolves to false when the user cancelled. */
  async signIn(): Promise<boolean> {
    if (this.signingIn) return false;
    this.signingIn = true;
    try {
      const account = await gggSignIn();
      if (account) this.account = account;
      return account !== null;
    } finally {
      this.signingIn = false;
    }
  }

  cancel() {
    void gggSignInCancel();
  }

  async signOut() {
    await gggSignOut();
    this.account = null;
  }
}

export const ggg = new GggStore();
