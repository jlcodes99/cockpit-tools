import { CSSProperties } from 'react';
import qoderIcon from '../../assets/icons/qoder.png';
import qoderIdeIcon from '../../assets/icons/qoder-ide.png';
import qoderCnIcon from '../../assets/icons/qoder-cn.png';
import qoderCnIdeIcon from '../../assets/icons/qoder-cn-ide.png';
import cliIcon from '../../assets/icons/antigravity-cli.svg';
import type { QoderClient, QoderRegion } from '../../utils/qoderRegion';

const icons = {
  global: { desktop: qoderIcon, ide: qoderIdeIcon, cli: cliIcon },
  cn: { desktop: qoderCnIcon, ide: qoderCnIdeIcon, cli: cliIcon },
} as const;

type QoderIconProps = {
  className?: string;
  style?: CSSProperties;
  region?: QoderRegion;
  client?: QoderClient;
};

export function QoderIcon({ className = 'nav-item-icon', style, region = 'global', client = 'desktop' }: QoderIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={icons[region][client]}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
