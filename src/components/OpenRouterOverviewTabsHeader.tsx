export type OpenRouterTab = 'overview' | 'models';

interface OpenRouterOverviewTabsHeaderProps {
  activeTab: OpenRouterTab;
  onTabChange: (tab: OpenRouterTab) => void;
  showModelsTab?: boolean;
}

export function OpenRouterOverviewTabsHeader({
  activeTab,
  onTabChange,
  showModelsTab,
}: OpenRouterOverviewTabsHeaderProps) {
  return (
    <div className="tab-bar">
      <button
        className={`tab-btn ${activeTab === 'overview' ? 'active' : ''}`}
        onClick={() => onTabChange('overview')}
      >
        Overview
      </button>
      {showModelsTab && (
        <button
          className={`tab-btn ${activeTab === 'models' ? 'active' : ''}`}
          onClick={() => onTabChange('models')}
        >
          Models
        </button>
      )}
    </div>
  );
}
