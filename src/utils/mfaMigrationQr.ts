import qrcode from 'qrcode-generator';
import { exportGoogleAuthenticatorMigrationUris, type MfaRecord } from './mfaVault';

export interface MfaMigrationQrPage {
  uri: string;
  count: number;
  svg: string;
  dataUrl: string;
}

export function buildMfaMigrationQrPages(records: readonly MfaRecord[]): MfaMigrationQrPage[] {
  return exportGoogleAuthenticatorMigrationUris(records).map(page => {
    try {
      const qr = qrcode(0, 'M');
      qr.addData(page.uri, 'Byte');
      qr.make();
      const svg = qr.createSvgTag({ cellSize: 4, margin: 16, scalable: false });
      return { ...page, svg, dataUrl: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}` };
    } catch {
      throw new Error('mfaVault.migrationQrFailed');
    }
  });
}
