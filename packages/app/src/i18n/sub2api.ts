export type Sub2APILocale = "zh" | "en";
export type Sub2APILanguagePreference = Sub2APILocale | "auto";

type LocaleResolutionInput = {
  explicitLocale?: string | null;
  intlLocale?: string | null;
  navigatorLanguages?: readonly string[] | null;
  navigatorLanguage?: string | null;
};

const DEFAULT_LOCALE: Sub2APILocale = "zh";
const FIAT_PAYMENT_PREFIXES = ["alipay", "wxpay"] as const;
const CRYPTO_PAYMENT_PREFIXES = ["usdt", "usdc"] as const;

export function normalizeSub2APILocale(value: string | null | undefined): Sub2APILocale {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) {
    return DEFAULT_LOCALE;
  }
  if (normalized.startsWith("zh")) {
    return "zh";
  }
  return "en";
}

export function normalizeSub2APILanguagePreference(
  value: string | null | undefined,
): Sub2APILanguagePreference {
  const normalized = value?.trim().toLowerCase();
  if (!normalized || normalized === "auto" || normalized === "system") {
    return "auto";
  }
  if (normalized.startsWith("zh")) {
    return "zh";
  }
  if (normalized.startsWith("en")) {
    return "en";
  }
  return "auto";
}

export function resolveSub2APILocale(input: LocaleResolutionInput = {}): Sub2APILocale {
  if (input.explicitLocale) {
    return normalizeSub2APILocale(input.explicitLocale);
  }

  if (input.intlLocale) {
    return normalizeSub2APILocale(input.intlLocale);
  }

  const navigatorLanguages = input.navigatorLanguages;
  if (navigatorLanguages && navigatorLanguages.length > 0) {
    return normalizeSub2APILocale(navigatorLanguages[0]);
  }

  if (input.navigatorLanguage) {
    return normalizeSub2APILocale(input.navigatorLanguage);
  }

  try {
    const locale = Intl.DateTimeFormat().resolvedOptions().locale;
    if (locale) {
      return normalizeSub2APILocale(locale);
    }
  } catch {
    // Use default below.
  }

  const runtimeNavigator =
    typeof navigator === "undefined"
      ? null
      : (navigator as Navigator & { languages?: readonly string[] });
  const firstNavigatorLanguage = runtimeNavigator?.languages?.[0] ?? runtimeNavigator?.language;
  return normalizeSub2APILocale(firstNavigatorLanguage);
}

export function resolveSub2APILocaleFromPreference(
  preference: string | null | undefined,
  input: LocaleResolutionInput = {},
): Sub2APILocale {
  const normalizedPreference = normalizeSub2APILanguagePreference(preference);
  if (normalizedPreference !== "auto") {
    return normalizedPreference;
  }
  return resolveSub2APILocale(input);
}

export function isSub2APIEnglish(locale: string | null | undefined): boolean {
  return normalizeSub2APILocale(locale) === "en";
}

function startsWithAny(value: string, prefixes: readonly string[]): boolean {
  return prefixes.some((prefix) => value.startsWith(prefix));
}

export function filterSub2APIPaymentTypesByLocale(
  types: readonly string[],
  locale: string | null | undefined,
): string[] {
  const normalizedLocale = normalizeSub2APILocale(locale);
  const prefixes = normalizedLocale === "en" ? CRYPTO_PAYMENT_PREFIXES : FIAT_PAYMENT_PREFIXES;
  return types.filter((type) => startsWithAny(type.trim().toLowerCase(), prefixes));
}

function stablecoinLabel(type: string, base: "USDT" | "USDC"): string {
  const [, network] = type.split(".");
  if (!network) {
    return base;
  }
  return `${base} (${network.charAt(0).toUpperCase()}${network.slice(1)})`;
}

export function getSub2APIPaymentLabel(
  paymentType: string,
  locale: string | null | undefined,
): string {
  const type = paymentType.trim().toLowerCase();
  const normalizedLocale = normalizeSub2APILocale(locale);
  if (type.startsWith("alipay")) {
    return normalizedLocale === "zh" ? "支付宝" : "Alipay";
  }
  if (type.startsWith("wxpay")) {
    return normalizedLocale === "zh" ? "微信支付" : "WeChat Pay";
  }
  if (type.startsWith("usdt")) {
    return stablecoinLabel(type, "USDT");
  }
  if (type.startsWith("usdc")) {
    return stablecoinLabel(type, "USDC");
  }
  if (type.startsWith("stripe")) {
    return "Stripe";
  }
  return paymentType;
}

export const sub2apiMessages = {
  zh: {
    common: {
      loading: "加载中…",
      retry: "重试",
    },
    pay: {
      addBalance: "充值余额",
      loadingRechargeOptions: "正在加载充值选项…",
      unableToLoadRechargeOptions: "无法加载充值选项",
      fullPayCenterFallback: "你仍然可以在浏览器中打开完整支付中心。",
      openFullPayCenter: "打开完整支付中心",
      rechargeAccount: "充值账户",
      accountFallback: "账户",
      currentBalance: "当前余额",
      rechargeAmount: "充值金额",
      creditedRange: (credited: string, min: string, max: string) =>
        `到账余额：${credited} · 可用范围：${min} - ${max}`,
      estimatedSettlement: (amount: string) => `预计结算：${amount}`,
      methodFee: (rate: number) => `支付手续费：${rate}%`,
      estimatedAmountToPay: (amount: string) => `预计实付金额：${amount}`,
      paymentMethod: "支付方式",
      methodUnavailable: "当前支付方式暂不可用。",
      remainingDailyQuota: (amount: string) => `今日剩余额度：${amount}`,
      stripeRuntimeHint: "当前运行环境只能在完整支付中心使用 Stripe 结账。",
      loadingCta: "加载中…",
      creatingOrder: "正在创建订单…",
      continueToPayment: "继续支付",
      signInAgain: "请重新登录后再创建支付订单。",
      selectPaymentMethod: "请先选择支付方式。",
      stripeFullCenterRequired: "Stripe 结账需要在完整支付中心打开。",
      pendingOrders: (count: number) => `你已有 ${count} 个待支付订单，请先完成或取消。`,
      enterValidAmount: "请输入有效的充值金额。",
      rechargeAmountRange: (min: number, max: number) => `充值金额需在 ${min} - ${max} 之间。`,
      failedLoadOrders: "加载订单概览失败。",
      paymentUserUnavailable: "支付用户信息不可用。",
      failedLoadConfig: "加载支付配置失败。",
      failedCreateOrder: "创建支付订单失败。",
      missingOrderId: "支付订单缺少订单号。",
      missingStatusAccessToken: "支付订单缺少状态访问凭证。",
      stripeWindowBlocked: "Stripe 支付窗口被拦截，请点击下方按钮重试。",
      stripeCouldNotOpen: "无法在当前环境打开 Stripe 结账，请使用完整支付中心。",
      paymentPollingTimedOut: "支付状态轮询超时，你可以返回充值页面后重试。",
      failedRefreshStatus: "刷新订单状态失败。",
      failedCancelOrder: "取消订单失败。",
      paymentSuccessfulToast: "支付成功，余额已更新。",
      order: "订单",
      timeRemaining: "剩余时间",
      expired: "已过期",
      amountToPay: "实付金额",
      credited: "到账",
      qrInstruction: (method: string) => `打开${method}并扫码完成支付`,
      openPaymentPage: "打开支付页面",
      redirectTitle: "支付将在浏览器中继续",
      redirectHint: "如果支付页面没有自动打开，请点击下方按钮重新打开。",
      stripeCheckout: "Stripe 结账",
      stripeCheckoutHint: "此订单需要安全的 Stripe 窗口；窗口打开时这里会继续轮询订单状态。",
      opening: "正在打开…",
      openSecurePaymentWindow: "打开安全支付窗口",
      cancelling: "正在取消…",
      cancelOrder: "取消订单",
      backToRecharge: "返回充值",
      status: {
        rechargeCompleteTitle: "充值完成",
        rechargeCompleteMessage: "余额已到账，正在关闭窗口…",
        paymentReceivedTitle: "已收到支付",
        paymentReceivedMessage: "已收到支付，余额充值仍在处理中。",
        rechargeUnfinishedTitle: "已支付，充值未完成",
        rechargeUnfinishedMessage: "支付已完成，但充值尚未完成，请稍后重试或联系客服。",
        awaitingPaymentTitle: "等待支付",
        awaitingPaymentMessage: "请使用下方方式完成支付，我们会自动刷新订单状态。",
        paymentFailedTitle: "支付失败",
        paymentFailedMessage: "本次支付未成功完成，你可以返回后重新创建订单。",
        orderCancelledTitle: "订单已取消",
        orderCancelledMessage: "该订单已在支付完成前取消。",
        orderExpiredTitle: "订单已超时",
        orderExpiredMessage: "该订单已超时，请创建新订单后重试。",
        statusUpdatedTitle: "支付状态已更新",
        statusUpdatedMessage: "支付状态已变化，如有需要可返回后重新创建订单。",
      },
    },
    modelCatalog: {
      title: "模型广场",
      signInHint: (cloudName: string) => `登录 ${cloudName} 后浏览模型广场。`,
      loading: "正在加载模型…",
      noModels: "暂无可用模型。",
      models: "模型",
      token: "Token",
      nonToken: "非 Token",
      bestSavings: "最高节省",
      group: "分组",
      allModels: "全部模型",
      platformModels: (count: number) => `${count} 个平台模型`,
      selectGroup: (platform: string) => `选择 ${platform} 分组`,
      selectGroupPlaceholder: "选择分组",
      search: "搜索",
      searchAllModels: "搜索全部模型",
      searchModelsInGroup: "搜索分组内模型",
      fallbackAcrossGroups: (platform: string) => `展示所有 ${platform} 分组的兜底视图。`,
      noSearchMatch: "没有匹配当前搜索的模型。",
      runtimeUnknown: "暂未观测到运行状态。",
      status: "状态",
      groups: (count: number) => `${count} 个分组`,
      groupDescription: (count: number, rate: number, status?: string) =>
        `${count} 个模型 · ${rate}x${status ? ` · ${status}` : ""}`,
    },
    modelCard: {
      actualPaid: "实付价格",
      balancePrice: "余额价格",
      groups: (count: number) => `${count} 个分组`,
      input: "输入 /MTok",
      output: "输出 /MTok",
      perRequest: "每次请求",
      perImage: "每张图片",
      savings: "节省",
      cachePricing: "缓存价格",
      write: "写入 /MTok",
      read: "读取 /MTok",
      promptCaching: "提示词缓存",
      alsoAvailableIn: "也可用于",
      savingsSuffix: "节省",
    },
    cloudPanel: {
      sections: {
        overview: "概览",
        keys: "API 密钥",
        routing: "路由",
        catalog: "模型广场",
        usage: "用量",
        status: "模型状态",
        referral: "邀请奖励",
      },
      browseOneSection: "一次浏览一个功能区。",
      accountTitle: "账户",
      accountFallback: "账户",
      signIn: "登录",
      signInBody: (cloudName: string, appName: string) =>
        `使用 GitHub 连接 ${cloudName}，管理账单、API 密钥、路由分组和模型广场。首次登录时，${appName} 会尝试自动补齐缺失的 Claude Code 或 Codex 路由；已有设备路由会保持不变，直到你主动切换密钥或分组。`,
      loginWithGitHub: "使用 GitHub 登录",
      connectedHint: (cloudName: string) =>
        `${cloudName} 会话已连接。你可以通过左侧功能区分别管理密钥、路由和模型。`,
      signedInAs: (account: string) => `已登录：${account}`,
      logout: "退出登录",
      cloudControlTitle: "云端控制",
      cloudControlHint: (cloudName: string) =>
        `先选择云端分组/路由；当路由需要密钥时，${cloudName} 会创建或复用 API 密钥。已运行的代理会继续使用当前路由，新会话会从模型选择器读取新的配置。`,
      createApiKey: "创建 API 密钥",
      switchGroup: "切换分组",
      viewModels: "查看模型",
      balanceUsageTitle: "余额与用量",
      retry: "重试",
      balance: "余额",
      recharge: "充值",
      today: "今日",
      week: "本周",
      month: "本月",
      requests: "次请求",
      currentRoutesTitle: "当前路由",
      notConfigured: "未配置",
      noActiveRoute: (scope: string) => `此设备尚未配置 ${scope} 路由。`,
      customRoute: "自定义路由",
      key: "密钥",
      group: "分组",
      none: "无",
      totalSpend: "总支出",
      quota: "额度",
      unlimitedQuota: "无限额度",
      routeUsageUnavailable: (cloudName: string) =>
        `此设备正在使用的路由未匹配当前 ${cloudName} 账户中的密钥，因此这里无法显示该路由的用量。`,
      chooseMatchingGroup: "重新选择分组",
      createMatchingApiKey: "创建 API 密钥",
      signInRequiredTitle: "需要登录",
      signInRequiredBody: (overview: string) =>
        `打开 ${overview} 登录后，再管理 API 密钥、路由、用量、模型状态或模型广场。`,
      goToOverview: "前往概览",
      sessionExpired: "会话已过期",
      loginAgainBeforePayment: "请重新登录后再打开支付。",
      unableOpenPayment: "无法打开支付",
    },
    setupCheck: {
      title: "环境检查",
      verifying: "正在验证配置...",
      review: "查看检查结果",
      continue: "继续",
      skip: "跳过",
      actions: {
        retry: "重试",
        manageRoutes: "管理路由",
        install: "安装",
        installAll: "全部安装",
        configure: "配置",
      },
      health: {
        label: "后端服务",
        checking: "正在检查服务连接...",
        reachable: "服务可访问",
        unreachable: "无法连接后端服务",
        unexpectedStatus: "服务返回了异常状态",
        connectionFailed: "连接失败",
      },
      routes: {
        apiKeyLabel: "API 密钥",
        cloudRoutesLabel: "云端路由",
        byokSkipped: "BYOK 模式，已跳过",
        checking: "正在检查 Claude/Codex 路由...",
        readyBoth: "当前云账户可创建 Claude Code 和 Codex 路由",
        claudeOnly: "Claude Code 路由可用。如需 Codex，请添加 OpenAI 分组或密钥。",
        codexOnly: "Codex 路由可用。如需 Claude Code，请添加 Anthropic 分组或密钥。",
        keysNotBound: "当前 API 密钥未绑定到 Claude Code 或 Codex 兼容路由",
        assignGroup: (cloudName: string) =>
          `继续前请先在 ${cloudName} 分配 anthropic 或 openai 分组。`,
        noCompatibleGroups: "此账户暂无 Claude Code 或 Codex 兼容分组",
        askGroupOrByok: "请申请 anthropic/openai 分组，或切换到 BYOK。",
        noManagedRoutes: "暂无可用的托管 Claude Code 或 Codex 路由",
        createRouteOrByok: "请创建兼容的云端路由，或改用 BYOK。",
        failedCheck: "检查 Claude/Codex 路由失败",
        notAuthenticated: "尚未登录",
        checkFailed: "检查失败",
      },
      cli: {
        label: "Claude Code & Codex",
        checking: "正在检查 CLI 工具...",
        desktopOnlySkipped: "仅桌面端需要，已跳过",
        toolsNotFullyInstalled: "CLI 工具尚未完整安装",
        missing: (items: string) => `缺少：${items}`,
        notInstalledError: "CLI 未安装",
        providersNotConfigured: "未配置 Claude Code 或 Codex 的提供商",
        providersNotConfiguredError: "提供商未配置",
        toolsInstalledButNotConfigured: "CLI 工具已安装但尚未配置",
        onlyConfigured: (configured: string) => `仅已配置 ${configured}`,
        providerNotConfigured: (unconfigured: string) => `${unconfigured} 提供商未配置`,
        partialConfigError: "部分配置缺失",
        installedAndConfigured: "Claude Code & Codex 已安装并配置",
        failedCheckConfiguration: "检查 CLI 配置失败",
        checkFailed: "检查失败",
        preparingInstall: "正在准备 CLI 安装...",
        stepReadyContinuing: (label: string) => `${label} 已就绪，继续...`,
        installationIncomplete: (items: string) => `安装未完成：${items}`,
        someToolsFailed: "部分工具安装失败",
        installFailedFallback: "安装失败。请重试或手动安装。",
        installFailureWithMissing: (base: string, items: string) =>
          `${base.replace(/[.。]$/, "")}。缺少：${items}`,
        installing: {
          git: "正在安装 Git Bash...",
          node: "正在安装 Node.js 22...",
          codex: "正在安装 Codex CLI...",
          claude: "正在安装 Claude Code CLI...",
        },
      },
    },
    loginScreen: {
      title: (appName: string) => `登录 ${appName}`,
      subtitle: (cloudName: string, appName: string) =>
        `请在浏览器中完成 ${cloudName} 登录。首次登录时，${appName} 会尝试为你补齐缺失的 Claude Code 和 Codex 路由，且不会覆盖此设备上已有的路由。`,
      envUrlInvalid:
        "EXPO_PUBLIC_MANAGED_SERVICE_URL 不是有效的 http(s) URL。请修正后重新构建或重启。",
      waitingForBrowser: "正在等待浏览器...",
      requestAuthenticationAgain: "重新发起认证",
      primaryLogin: "登录",
      byokCaption: "已经有自己的 API 密钥？",
      useByokInstead: "改用 BYOK →",
    },
    modeSelect: {
      title: "你想如何连接？",
      subtitle: (cloudName: string) => `使用 ${cloudName}，或自带 API 密钥（BYOK）访问模型。`,
      cloudDescription: "登录后使用托管 Claude Code / Codex 路由、按量计费和快速配置。",
      signInMeta: "登录 · 推荐",
      byokDescription: "使用你自己的 Anthropic / OpenAI 密钥，并在设置中添加提供商。无需云端登录。",
      noSignInMeta: "无需登录",
      recommended: "推荐",
    },
    authAlerts: {
      sessionExpired: "会话已过期",
      loginAgain: "请重新登录。",
      unableOpenPayment: "无法打开支付",
      signedIn: "已登录",
      autoRouteSetupFailed: (message: string) =>
        `账户已连接，但自动配置 Claude/Codex 未完成。${message}`,
      loginFailed: "登录失败",
    },
    sidebar: {
      projects: "项目",
      chat: "聊天",
      noHost: "无主机",
      collapseAllProjects: "折叠或展开所有项目",
      addProject: "添加项目",
      newChat: "新建聊天",
      settings: "设置",
      sort: "排序",
      sortByProject: "按项目",
      sortByTime: "按时间",
      switchHost: "切换主机",
      searchHosts: "搜索主机...",
    },
    sidebarUser: {
      account: "账户",
      userMenu: "用户菜单",
      sessionExpired: "会话已过期",
      loginAgain: "请重新登录。",
      unableOpenPayment: "无法打开支付",
      balance: "余额",
      todayUsage: (cost: string, requests: number) => `今日：${cost}（${requests} 次请求）`,
      recharge: "充值",
      apiKeys: "API 密钥",
      routingGroups: "路由分组",
      settings: "设置",
      logout: "退出登录",
    },
    sidebarWorkspace: {
      newWorktree: "新建 worktree",
      createNewWorktree: (name: string) => `为 ${name} 新建 worktree`,
      projectActions: "项目操作",
      renameProject: "重命名项目",
      deleteProject: "删除项目",
      deleting: "删除中...",
      noProjectsYet: "暂无项目",
      addProjectToStart: "添加项目后开始",
      addProject: "添加项目",
      renamePrompt: "重命名项目",
      removeProjectTitle: "移除项目？",
      removeProjectMessage: (name: string) => `从侧边栏移除 "${name}"？\n\n磁盘文件不会被修改。`,
      remove: "移除",
      cancel: "取消",
      hostNotConnected: "主机未连接",
      failedRemoveSomeWorkspaces: "部分工作区移除失败",
      scriptsAvailable: "有可用脚本",
      creating: "创建中...",
      workspaceActions: "工作区操作",
      copyPath: "复制路径",
      copyBranchName: "复制分支名",
      archive: "归档",
      archiveWorktree: "归档 worktree",
      hideFromSidebar: "从侧边栏隐藏",
      archiving: "归档中...",
      hiding: "隐藏中...",
      archiveWorktreeTitle: "归档 worktree？",
      archiveWorktreeMessage: (name: string) =>
        `归档 "${name}"？\n\n该 worktree 会从磁盘移除，终端会停止，其中的代理会被归档。\n\n如果已经提交，分支仍然可以访问。`,
      workspacePathUnavailable: "工作区路径不可用",
      failedArchiveWorktree: "归档 worktree 失败",
      hideWorkspaceTitle: "隐藏工作区？",
      hideWorkspaceMessage: (name: string) => `从侧边栏隐藏 "${name}"？\n\n磁盘文件不会被修改。`,
      hide: "隐藏",
      failedHideWorkspace: "隐藏工作区失败",
      pathCopied: "路径已复制",
      branchNameCopied: "分支名已复制",
    },
    paseoCloudApiKeys: {
      title: "API 密钥（高级）",
      sectionHint: (platform: string, cliLabel: string) =>
        `用于 ${platform} 路由的高级密钥管理。常规流程中，选择 Cloud 分组会自动为 ${cliLabel} 创建或复用密钥。`,
      createApiKey: "创建 API 密钥",
      pageHint: "此页面只过滤和管理密钥。只有点击“设为全局 CLI 默认值”后才会更改 CLI 路由。",
      setGlobalDefault: "设为全局 CLI 默认值",
      noCompatibleGroupsTitle: "暂无兼容分组",
      noCompatibleGroupsBody: (platform: string, cliLabel: string, cloudName: string) =>
        `当前账户没有可用于 ${cliLabel} 的 ${platform} 分组。请在 ${cloudName} 中添加兼容分组，或为此 CLI 使用 BYOK。`,
      viewScopeInstead: (cliLabel: string) => `改看 ${cliLabel}`,
      search: "搜索",
      searchPlaceholder: (platform: string) => `按名称或分组搜索 ${platform} 密钥`,
      allGroups: "全部分组",
      showAllKeys: (cliLabel: string) => `显示全部 ${cliLabel} 密钥`,
      filterByGroup: "按分组过滤",
      filterByGroupTitle: (cliLabel: string) => `按分组过滤 ${cliLabel} 密钥`,
      retry: "重试",
      loadingKeys: "正在加载密钥…",
      noKeysMatch: (cliLabel: string) =>
        `没有匹配当前筛选条件的 ${cliLabel} 密钥。你可以在上方创建密钥，或清除筛选条件。`,
      group: "分组",
      none: "无",
      used: "已用",
      quota: "额度",
      unlimited: "无限",
      advancedAction: "高级操作：写入",
      activeForCli: (cliLabel: string) => `此设备上的 ${cliLabel} 正在使用`,
      groupInactive: "此密钥的分组当前未启用。使用前请先切换分组。",
      edit: "编辑",
      delete: "删除",
      applying: "正在应用…",
      activeCta: (cliLabel: string) => `已启用 · ${cliLabel}`,
      modalTitle: (editing: boolean) => (editing ? "编辑 API 密钥" : "创建 API 密钥"),
      modalHintEdit: (platform: string, cliLabel: string) =>
        `更新用于 ${cliLabel} 的 ${platform} 密钥信息。`,
      modalHintCreate: (platform: string, cliLabel: string) =>
        `为 ${cliLabel} 创建新的 ${platform} 密钥。`,
      name: "名称",
      keyName: "密钥名称",
      groupLabel: "分组",
      selectGroupTitle: (cliLabel: string) => `为 ${cliLabel} 密钥选择分组`,
      selectGroupPlaceholder: (platform: string) => `选择 ${platform} 分组…`,
      cancel: "取消",
      save: "保存",
      create: "创建",
      saving: "保存中…",
      serviceEndpointInvalid: "服务端点无效。",
      missingNameTitle: "缺少名称",
      missingNameBody: "请输入密钥名称。",
      missingGroupTitle: "缺少分组",
      missingGroupBody: "请选择此密钥使用的分组。",
      updateFailed: "更新失败",
      createFailed: "创建密钥失败",
      cannotApplyKey: "无法应用密钥",
      movedTitle: "已切换到匹配的标签",
      movedKeyMessage: (keyName: string, cliLabel: string, cloudName: string) =>
        `密钥 "${keyName}" 属于 ${cliLabel}。${cloudName} 已为你切换标签页，你可以在那里应用它。`,
      globalUpdatedTitle: "全局 CLI 默认值已更新",
      globalKeyUpdatedMessage: (
        cloudName: string,
        keyName: string,
        cliLabel: string,
        configTarget: string,
      ) => `${cloudName} 密钥 "${keyName}" 现在仅配置 ${cliLabel}。已更新 ${configTarget}。`,
      switchFailed: "切换失败",
      deleteFailed: "删除失败",
    },
    paseoCloudRouting: {
      title: "路由分组",
      hint: "选择 Cloud 分组即可开始使用路由。此设置页会把分组设为全局 CLI 默认值；新建代理时的模型选择器可以只为当前工作区指定分组。",
      currentTab: (cliLabel: string, platform: string) =>
        `当前标签：${cliLabel}，仅显示 ${platform} 分组。`,
      retry: "重试",
      loadingGroups: "正在加载分组…",
      noGroupsTitle: "暂无可用路由分组",
      noGroupsBody: (platform: string, cliLabel: string, cloudName: string) =>
        `当前账户没有暴露可用于 ${cliLabel} 的 ${platform} 路由分组。请在 ${cloudName} 中添加兼容分组，或为此 CLI 使用 BYOK。`,
      viewScopeRoutes: (cliLabel: string) => `查看 ${cliLabel} 路由`,
      recommended: "推荐",
      latency: "延迟",
      runtimeUnknown: "此分组尚未观测到运行状态。",
      activeRouteVia: (key: string) => `当前路由使用 ${key}`,
      reusableKeyCount: (count: number) => `${count} 个密钥可复用`,
      noExistingKey: "还没有现成密钥。应用此分组时会自动创建一个。",
      recommendedInsight: (cliLabel: string) =>
        `基于健康度、延迟和价格，这是当前最适合 ${cliLabel} 的可用选项。`,
      downWarning: "近期探测结果不健康。除非确实需要此路由，否则建议选择其他分组。",
      advancedAction: "高级操作：写入",
      applying: "正在应用…",
      activeCta: (cliLabel: string) => `已启用 · ${cliLabel}`,
      setGlobalDefault: "设为全局 CLI 默认值",
      serviceEndpointInvalid: "服务端点无效。",
      cannotUseGroup: "无法使用分组",
      groupUnavailable: "所选分组已不可用。",
      movedTitle: "已切换到匹配的标签",
      movedGroupMessage: (groupName: string, cliLabel: string, cloudName: string) =>
        `分组 "${groupName}" 属于 ${cliLabel}。${cloudName} 已为你切换标签页，你可以在那里应用它。`,
      defaultKeyName: (groupName: string) => `${groupName} 密钥`,
      globalUpdatedTitle: "全局 CLI 默认值已更新",
      globalGroupUpdatedMessage: (groupName: string, cliLabel: string, configTarget: string) =>
        `分组 "${groupName}" 现在仅配置 ${cliLabel}。已更新 ${configTarget}。新的工作区会话也可以通过模型选择器使用分组路由，而无需更改此全局默认值。`,
      switchFailed: "切换失败",
    },
    paseoCloudUsage: {
      title: "用量",
      datePresets: {
        today: "今日",
        "7d": "7 天",
        "30d": "30 天",
      },
      requestTypes: {
        ws: "WS",
        stream: "流式",
        sync: "同步",
        unknown: "未知",
      },
      billingModes: {
        perRequest: "按请求",
        image: "图片",
        token: "Token",
      },
      unknownKey: "未知密钥",
      keyNumber: (id: number) => `密钥 #${id}`,
      allApiKeys: "全部 API 密钥",
      unknownModel: "未知模型",
      tokens: "Token",
      cache: "缓存",
      cost: "费用",
      latency: "延迟",
      input: "输入",
      output: "输出",
      write: "写入",
      read: "读取",
      standard: "标准",
      first: "首 Token",
      noInboundEndpoint: "未记录入口端点",
      signInHint: (cloudName: string) => `登录 ${cloudName} 后查看用量。`,
      rangeTitle: "用量范围",
      rangeText: (start: string, end: string) => `${start} 至 ${end}`,
      refresh: "刷新",
      requests: "请求",
      recordsInRange: (count: number) => `范围内 ${count} 条记录`,
      actualCost: "实际费用",
      avgDuration: "平均耗时",
      acrossMatchedRequests: "匹配请求的平均值",
      recordsTitle: "用量记录",
      pageOf: (page: number, pages: number) => `第 ${page} 页 / 共 ${pages} 页`,
      prev: "上一页",
      next: "下一页",
      loading: "正在加载用量…",
      noRecords: "此范围内暂无用量记录。",
    },
    paseoCloudModelStatus: {
      title: "模型状态",
      signInHint: (cloudName: string) => `登录 ${cloudName} 后查看模型运行状态。`,
      runtimeHealth: "运行健康度",
      refreshInterval: "此功能区打开时每 30 秒自动刷新。",
      refresh: "刷新",
      groups: "分组",
      loadingGroups: "正在加载分组状态…",
      noGroups: (cloudName: string) => `${cloudName} 暂未返回受监控分组。`,
      statusLabels: {
        up: "健康",
        degraded: "降级",
        down: "不可用",
        unknown: "未知",
      },
      observed: "观测于",
      latency: "延迟",
      availability: "可用性",
      average: "平均",
      probes: "次探测",
      detailHint: "历史、近期探测记录和状态事件。",
      loadingDetails: "正在加载状态详情…",
      availabilityHistory: "可用性历史",
      noHistory: "暂无历史桶。",
      recentProbes: "近期探测",
      noRecords: "暂无探测记录。",
      statusEvents: "状态事件",
      noEvents: "暂无状态事件。",
      statusFallback: "状态",
      unknownStatus: "未知",
      http: "HTTP",
    },
    paseoCloudReferral: {
      title: "邀请奖励",
      loading: "正在加载邀请信息…",
      referralCode: "邀请码",
      copy: "复制",
      copied: "已复制",
      referralLink: "邀请链接",
      share: "分享",
      total: "总计",
      rewarded: "已奖励",
      pending: "待确认",
      earned: "已获得",
      balance: "余额",
      subscriptionDays: (days: number) => `${days} 天订阅`,
      referrerReward: (balance: string, subscription: string | null) =>
        `邀请人：${balance} 余额${subscription ? ` + ${subscription}` : ""}`,
      refereeReward: (balance: string, subscription: string | null) =>
        `被邀请人：${balance} 余额${subscription ? ` + ${subscription}` : ""}`,
      historyTitle: "邀请历史",
      statusLabel: (status: string) => {
        if (status === "rewarded") return "已奖励";
        if (status === "pending") return "待确认";
        return status;
      },
    },
    settings: {
      title: "设置",
      loadingSettings: "正在加载设置…",
      back: "返回",
      local: "本地",
      addHost: "添加主机",
      sections: {
        general: "通用",
        paseoCloud: "云服务",
        managedProvider: "提供商",
        shortcuts: "快捷键",
        integrations: "集成",
        permissions: "权限",
        diagnostics: "诊断",
        about: "关于",
      },
      general: "通用",
      theme: "主题",
      language: "语言",
      languageHint: "用于客户端界面、云服务、支付和模型广场",
      defaultSend: "默认发送",
      defaultSendHint: "当代理正在运行时，按 Enter 后的处理方式",
      interrupt: "中断",
      queue: "排队",
      themes: {
        light: "浅色",
        dark: "深色",
        zinc: "锌灰",
        midnight: "午夜",
        claude: "Claude",
        ghostty: "Ghostty",
        auto: "跟随系统",
      },
      languages: {
        auto: "跟随系统",
        zh: "中文",
        en: "English",
      },
      diagnostics: "诊断",
      testAudio: "测试音频",
      playing: "播放中…",
      playTest: "播放测试",
      playbackFailed: (message: string) => `播放失败：${message}`,
      about: "关于",
      version: "版本",
      releaseChannel: "发布通道",
      releaseChannelHint: "切换到 Beta 可更早获得更新并参与反馈",
      stable: "稳定版",
      beta: "Beta",
      appUpdates: "应用更新",
      readyToInstall: (version: string) => `可安装：${version}`,
      check: "检查",
      checking: "检查中…",
      update: "更新",
      updateTo: (version: string) => `更新到 ${version}`,
      installing: "安装中…",
      installDesktopUpdate: "安装桌面更新",
      installDesktopUpdateMessage: (appName: string) => `这会更新此电脑上的 ${appName}`,
      installUpdate: "安装更新",
      cancel: "取消",
      error: "错误",
      unableOpenUpdateConfirmation: "无法打开更新确认对话框。",
      managedProviderTitle: "此设备",
      managedProviderBody:
        "此电脑上 Claude Code 和 Codex 的活跃 API 路由，以及已保存和自定义的端点。你可以让每个 CLI 使用不同的已保存条目。",
      managedProviderCloudHint: (cloudName: string) =>
        `${cloudName}（账户、密钥、账单）位于侧边栏的 ${cloudName} 下。`,
      paseoCloudBody:
        "托管服务的账户登录、余额、路由分组、API 密钥和模型广场。它与 Provider 下的本机 Claude/Codex 路由分开管理。",
    },
  },
  en: {
    common: {
      loading: "Loading…",
      retry: "Retry",
    },
    pay: {
      addBalance: "Add balance",
      loadingRechargeOptions: "Loading recharge options…",
      unableToLoadRechargeOptions: "Unable to load recharge options",
      fullPayCenterFallback: "You can still open the full payment center in your browser.",
      openFullPayCenter: "Open full pay center",
      rechargeAccount: "Recharge account",
      accountFallback: "Account",
      currentBalance: "Current balance",
      rechargeAmount: "Recharge amount",
      creditedRange: (credited: string, min: string, max: string) =>
        `Credited to balance: ${credited} · Allowed range: ${min} - ${max}`,
      estimatedSettlement: (amount: string) => `Estimated settlement: ${amount}`,
      methodFee: (rate: number) => `Method fee: ${rate}%`,
      estimatedAmountToPay: (amount: string) => `Estimated amount to pay: ${amount}`,
      paymentMethod: "Payment method",
      methodUnavailable: "This payment method is currently unavailable.",
      remainingDailyQuota: (amount: string) => `Remaining daily quota: ${amount}`,
      stripeRuntimeHint:
        "Stripe checkout is only available in the full payment center in this runtime.",
      loadingCta: "Loading…",
      creatingOrder: "Creating order…",
      continueToPayment: "Continue to payment",
      signInAgain: "Please sign in again before creating a payment order.",
      selectPaymentMethod: "Select a payment method first.",
      stripeFullCenterRequired: "Stripe checkout needs the full payment center in this runtime.",
      pendingOrders: (count: number) =>
        `You already have ${count} pending orders. Please complete or cancel them first.`,
      enterValidAmount: "Enter a valid recharge amount.",
      rechargeAmountRange: (min: number, max: number) =>
        `Recharge amount must be between ${min} and ${max}.`,
      failedLoadOrders: "Failed to load order summary.",
      paymentUserUnavailable: "Payment user info is unavailable.",
      failedLoadConfig: "Failed to load payment config.",
      failedCreateOrder: "Failed to create payment order.",
      missingOrderId: "Payment order is missing an order id.",
      missingStatusAccessToken: "Payment order is missing a status access token.",
      stripeWindowBlocked:
        "Stripe payment window was blocked. Use the button below to try opening it again.",
      stripeCouldNotOpen:
        "Stripe checkout could not open here. Please use the full payment center.",
      paymentPollingTimedOut:
        "Payment status polling timed out. You can go back to recharge and try again if needed.",
      failedRefreshStatus: "Failed to refresh order status.",
      failedCancelOrder: "Failed to cancel this order.",
      paymentSuccessfulToast: "Payment successful. Balance updated.",
      order: "Order",
      timeRemaining: "Time remaining",
      expired: "Expired",
      amountToPay: "Amount to pay",
      credited: "Credited",
      qrInstruction: (method: string) => `Open ${method} and scan to complete payment`,
      openPaymentPage: "Open payment page",
      redirectTitle: "Payment continues in your browser",
      redirectHint:
        "If the payment page did not open automatically, use the button below to reopen it.",
      stripeCheckout: "Stripe checkout",
      stripeCheckoutHint:
        "A secure Stripe window is required for this order. We keep polling the order here while that window is open.",
      opening: "Opening…",
      openSecurePaymentWindow: "Open secure payment window",
      cancelling: "Cancelling…",
      cancelOrder: "Cancel order",
      backToRecharge: "Back to recharge",
      status: {
        rechargeCompleteTitle: "Recharge complete",
        rechargeCompleteMessage: "Your balance has been credited. Closing this dialog…",
        paymentReceivedTitle: "Payment received",
        paymentReceivedMessage:
          "Your payment was received and the balance top-up is still being processed.",
        rechargeUnfinishedTitle: "Payment received, recharge not finished",
        rechargeUnfinishedMessage:
          "Payment completed, but the recharge has not finished yet. Please try again later or contact support.",
        awaitingPaymentTitle: "Awaiting payment",
        awaitingPaymentMessage:
          "Complete payment using the method below. We will refresh this order automatically.",
        paymentFailedTitle: "Payment failed",
        paymentFailedMessage:
          "This payment did not complete successfully. You can go back and create a new order.",
        orderCancelledTitle: "Order cancelled",
        orderCancelledMessage: "This order was cancelled before payment completed.",
        orderExpiredTitle: "Order expired",
        orderExpiredMessage:
          "This order expired before payment completed. Create a new order to try again.",
        statusUpdatedTitle: "Payment status updated",
        statusUpdatedMessage:
          "The payment status changed. You can go back and create a new order if needed.",
      },
    },
    modelCatalog: {
      title: "Model catalog",
      signInHint: (cloudName: string) => `Sign in to ${cloudName} to browse the model catalog.`,
      loading: "Loading catalog...",
      noModels: "No models available.",
      models: "Models",
      token: "Token",
      nonToken: "Non-token",
      bestSavings: "Best savings",
      group: "Group",
      allModels: "All models",
      platformModels: (count: number) => `${count} platform models`,
      selectGroup: (platform: string) => `Select ${platform} group`,
      selectGroupPlaceholder: "Select group",
      search: "Search",
      searchAllModels: "Search all models",
      searchModelsInGroup: "Search models in group",
      fallbackAcrossGroups: (platform: string) => `Fallback view across all ${platform} groups.`,
      noSearchMatch: "No models match this search.",
      runtimeUnknown: "Runtime status has not been observed yet.",
      status: "Status",
      groups: (count: number) => `${count} groups`,
      groupDescription: (count: number, rate: number, status?: string) =>
        `${count} models · ${rate}x${status ? ` · ${status}` : ""}`,
    },
    modelCard: {
      actualPaid: "Actual Paid",
      balancePrice: "Balance Price",
      groups: (count: number) => `${count} groups`,
      input: "Input /MTok",
      output: "Output /MTok",
      perRequest: "Per Request",
      perImage: "Per Image",
      savings: "Savings",
      cachePricing: "Cache Pricing",
      write: "Write /MTok",
      read: "Read /MTok",
      promptCaching: "prompt caching",
      alsoAvailableIn: "Also available in",
      savingsSuffix: "savings",
    },
    cloudPanel: {
      sections: {
        overview: "Overview",
        keys: "API Keys",
        routing: "Routing",
        catalog: "Model Catalog",
        usage: "Usage",
        status: "Model Status",
        referral: "Referral",
      },
      browseOneSection: "Browse one section at a time.",
      accountTitle: "Account",
      accountFallback: "Account",
      signIn: "Sign in",
      signInBody: (cloudName: string, appName: string) =>
        `Connect with GitHub for ${cloudName} billing, API keys, routing groups, and the model catalog. On first sign-in, ${appName} tries to fill in any missing Claude Code or Codex route automatically. Existing device routes stay unchanged until you explicitly switch a key or group.`,
      loginWithGitHub: "Login with GitHub",
      connectedHint: (cloudName: string) =>
        `Your ${cloudName} session is connected. Use the sections on the left to manage keys, routing, and models without mixing those controls together.`,
      signedInAs: (account: string) => `Signed in as ${account}`,
      logout: "Logout",
      cloudControlTitle: "Cloud control",
      cloudControlHint: (cloudName: string) =>
        `Choose a Cloud group/route first; ${cloudName} will create or reuse an API key when a route needs one. Existing running agents keep their current route, and new sessions pick up changes from the model selector.`,
      createApiKey: "Create API key",
      switchGroup: "Switch group",
      viewModels: "View models",
      balanceUsageTitle: "Balance & usage",
      retry: "Retry",
      balance: "Balance",
      recharge: "Recharge",
      today: "Today",
      week: "Week",
      month: "Month",
      requests: "requests",
      currentRoutesTitle: "Current routes",
      notConfigured: "Not configured",
      noActiveRoute: (scope: string) => `This device does not have an active ${scope} route yet.`,
      customRoute: "Custom route",
      key: "Key",
      group: "Group",
      none: "none",
      totalSpend: "Total spend",
      quota: "Quota",
      unlimitedQuota: "Unlimited quota",
      routeUsageUnavailable: (cloudName: string) =>
        `This device is using a route that does not match a key in the current ${cloudName} account, so per-route usage is unavailable here.`,
      chooseMatchingGroup: "Choose group",
      createMatchingApiKey: "Create API key",
      signInRequiredTitle: "Sign in required",
      signInRequiredBody: (overview: string) =>
        `Open ${overview} to sign in before managing API keys, routing, usage, model status, or the model catalog.`,
      goToOverview: "Go to Overview",
      sessionExpired: "Session expired",
      loginAgainBeforePayment: "Please log in again before opening payment.",
      unableOpenPayment: "Unable to open payment",
    },
    setupCheck: {
      title: "Environment Check",
      verifying: "Verifying your configuration...",
      review: "Review the results below",
      continue: "Continue",
      skip: "Skip",
      actions: {
        retry: "Retry",
        manageRoutes: "Manage Routes",
        install: "Install",
        installAll: "Install All",
        configure: "Configure",
      },
      health: {
        label: "Backend Service",
        checking: "Checking service reachability...",
        reachable: "Service is reachable",
        unreachable: "Cannot reach backend service",
        unexpectedStatus: "Service returned unexpected status",
        connectionFailed: "Connection failed",
      },
      routes: {
        apiKeyLabel: "API Key",
        cloudRoutesLabel: "Cloud routes",
        byokSkipped: "BYOK mode — skipped",
        checking: "Checking Claude/Codex routes...",
        readyBoth: "Claude Code and Codex routes can be created from your current cloud account",
        claudeOnly:
          "Claude Code routing is available. Add an OpenAI group or key if you also want Codex.",
        codexOnly:
          "Codex routing is available. Add an Anthropic group or key if you also want Claude Code.",
        keysNotBound:
          "Your current API keys are not bound to Claude Code or Codex compatible routes",
        assignGroup: (cloudName: string) =>
          `Assign an anthropic or openai group in ${cloudName} before continuing.`,
        noCompatibleGroups:
          "No Claude Code or Codex compatible groups are available for this account",
        askGroupOrByok: "Ask for an anthropic/openai group or switch to BYOK.",
        noManagedRoutes: "No managed Claude Code or Codex routes are available yet",
        createRouteOrByok: "Create a compatible cloud route or use BYOK instead.",
        failedCheck: "Failed to check Claude/Codex routes",
        notAuthenticated: "Not authenticated",
        checkFailed: "Check failed",
      },
      cli: {
        label: "Claude Code & Codex",
        checking: "Checking CLI tools...",
        desktopOnlySkipped: "Desktop only — skipped",
        toolsNotFullyInstalled: "CLI tools not fully installed",
        missing: (items: string) => `Missing: ${items}`,
        notInstalledError: "CLI not installed",
        providersNotConfigured: "No providers configured for Claude Code or Codex",
        providersNotConfiguredError: "Providers not configured",
        toolsInstalledButNotConfigured: "CLI tools installed but not configured",
        onlyConfigured: (configured: string) => `Only ${configured} configured`,
        providerNotConfigured: (unconfigured: string) => `${unconfigured} provider not configured`,
        partialConfigError: "Partial config",
        installedAndConfigured: "Claude Code & Codex installed and configured",
        failedCheckConfiguration: "Failed to check CLI configuration",
        checkFailed: "Check failed",
        preparingInstall: "Preparing CLI installation...",
        stepReadyContinuing: (label: string) => `${label} ready. Continuing...`,
        installationIncomplete: (items: string) => `Installation incomplete: ${items}`,
        someToolsFailed: "Some tools failed to install",
        installFailedFallback: "Install failed. Please retry or install manually.",
        installFailureWithMissing: (base: string, items: string) =>
          `${base.replace(/\.$/, "")}. Missing: ${items}`,
        installing: {
          git: "Installing Git Bash...",
          node: "Installing Node.js 22...",
          codex: "Installing Codex CLI...",
          claude: "Installing Claude Code CLI...",
        },
      },
    },
    loginScreen: {
      title: (appName: string) => `Sign in to ${appName}`,
      subtitle: (cloudName: string, appName: string) =>
        `Complete sign-in in your browser for ${cloudName}. On first sign-in, ${appName} tries to configure any missing Claude Code and Codex routes for you without replacing routes that already exist on this device.`,
      envUrlInvalid:
        "EXPO_PUBLIC_MANAGED_SERVICE_URL is not a valid http(s) URL. Fix it and rebuild or restart.",
      waitingForBrowser: "Waiting for browser...",
      requestAuthenticationAgain: "Request authentication again",
      primaryLogin: "Sign in",
      byokCaption: "Already have your own API keys?",
      useByokInstead: "Use BYOK instead →",
    },
    modeSelect: {
      title: "How do you want to connect?",
      subtitle: (cloudName: string) =>
        `Use ${cloudName}, or bring your own API keys (BYOK) for model access.`,
      cloudDescription:
        "Sign in for managed Claude Code / Codex routing, usage-based billing, and quick setup.",
      signInMeta: "Sign in · Recommended",
      byokDescription:
        "Use your own Anthropic / OpenAI keys and add providers in Settings. No cloud sign-in.",
      noSignInMeta: "No sign-in",
      recommended: "Recommended",
    },
    authAlerts: {
      sessionExpired: "Session expired",
      loginAgain: "Please log in again.",
      unableOpenPayment: "Unable to open payment",
      signedIn: "Signed in",
      autoRouteSetupFailed: (message: string) =>
        `Your account is connected, but automatic Claude/Codex setup did not finish. ${message}`,
      loginFailed: "Login failed",
    },
    sidebar: {
      projects: "Projects",
      chat: "Chat",
      noHost: "No host",
      collapseAllProjects: "Collapse or expand all projects",
      addProject: "Add project",
      newChat: "New chat",
      settings: "Settings",
      sort: "Sort",
      sortByProject: "By project",
      sortByTime: "By time",
      switchHost: "Switch host",
      searchHosts: "Search hosts...",
    },
    sidebarUser: {
      account: "Account",
      userMenu: "User menu",
      sessionExpired: "Session expired",
      loginAgain: "Please log in again.",
      unableOpenPayment: "Unable to open payment",
      balance: "Balance",
      todayUsage: (cost: string, requests: number) => `Today: ${cost} (${requests} req)`,
      recharge: "Recharge",
      apiKeys: "API keys",
      routingGroups: "Routing groups",
      settings: "Settings",
      logout: "Logout",
    },
    sidebarWorkspace: {
      newWorktree: "New worktree",
      createNewWorktree: (name: string) => `Create a new worktree for ${name}`,
      projectActions: "Project actions",
      renameProject: "Rename project",
      deleteProject: "Delete project",
      deleting: "Deleting...",
      noProjectsYet: "No projects yet",
      addProjectToStart: "Add a project to get started",
      addProject: "Add project",
      renamePrompt: "Rename project",
      removeProjectTitle: "Remove project?",
      removeProjectMessage: (name: string) =>
        `Remove "${name}" from the sidebar?\n\nFiles on disk will not be changed.`,
      remove: "Remove",
      cancel: "Cancel",
      hostNotConnected: "Host is not connected",
      failedRemoveSomeWorkspaces: "Failed to remove some workspaces",
      scriptsAvailable: "Scripts available",
      creating: "Creating...",
      workspaceActions: "Workspace actions",
      copyPath: "Copy path",
      copyBranchName: "Copy branch name",
      archive: "Archive",
      archiveWorktree: "Archive worktree",
      hideFromSidebar: "Hide from sidebar",
      archiving: "Archiving...",
      hiding: "Hiding...",
      archiveWorktreeTitle: "Archive worktree?",
      archiveWorktreeMessage: (name: string) =>
        `Archive "${name}"?\n\nThe worktree will be removed from disk, terminals will be stopped, and agents inside will be archived.\n\nYour branch is still accessible if you committed.`,
      workspacePathUnavailable: "Workspace path not available",
      failedArchiveWorktree: "Failed to archive worktree",
      hideWorkspaceTitle: "Hide workspace?",
      hideWorkspaceMessage: (name: string) =>
        `Hide "${name}" from the sidebar?\n\nFiles on disk will not be changed.`,
      hide: "Hide",
      failedHideWorkspace: "Failed to hide workspace",
      pathCopied: "Path copied",
      branchNameCopied: "Branch name copied",
    },
    paseoCloudApiKeys: {
      title: "API Keys (advanced)",
      sectionHint: (platform: string, cliLabel: string) =>
        `Advanced key management for ${platform} routes. In the normal flow, choosing a Cloud group automatically creates or reuses a key for ${cliLabel}.`,
      createApiKey: "Create API key",
      pageHint:
        "This page only filters and manages keys. It does not change CLI routing until you press Set as global CLI default.",
      setGlobalDefault: "Set as global CLI default",
      noCompatibleGroupsTitle: "No compatible groups yet",
      noCompatibleGroupsBody: (platform: string, cliLabel: string, cloudName: string) =>
        `Your current account does not have any ${platform} groups available for ${cliLabel}. Add a compatible group in ${cloudName}, or use BYOK for this CLI.`,
      viewScopeInstead: (cliLabel: string) => `View ${cliLabel} instead`,
      search: "Search",
      searchPlaceholder: (platform: string) => `Search ${platform} keys by name or group`,
      allGroups: "All groups",
      showAllKeys: (cliLabel: string) => `Show all ${cliLabel} keys`,
      filterByGroup: "Filter by group",
      filterByGroupTitle: (cliLabel: string) => `Filter ${cliLabel} keys by group`,
      retry: "Retry",
      loadingKeys: "Loading keys…",
      noKeysMatch: (cliLabel: string) =>
        `No ${cliLabel} keys match this filter. Create a key above, or clear the current filters.`,
      group: "Group",
      none: "none",
      used: "Used",
      quota: "Quota",
      unlimited: "Unlimited",
      advancedAction: "Advanced action: writes",
      activeForCli: (cliLabel: string) => `Active for ${cliLabel} on this device`,
      groupInactive: "This key's group is currently inactive. Switch groups before using it.",
      edit: "Edit",
      delete: "Delete",
      applying: "Applying…",
      activeCta: (cliLabel: string) => `Active · ${cliLabel}`,
      modalTitle: (editing: boolean) => (editing ? "Edit API key" : "Create API key"),
      modalHintEdit: (platform: string, cliLabel: string) =>
        `Update the ${platform} key details used for ${cliLabel}.`,
      modalHintCreate: (platform: string, cliLabel: string) =>
        `Create a new ${platform} key for ${cliLabel}.`,
      name: "Name",
      keyName: "Key name",
      groupLabel: "Group",
      selectGroupTitle: (cliLabel: string) => `Select group for ${cliLabel} key`,
      selectGroupPlaceholder: (platform: string) => `Select a ${platform} group…`,
      cancel: "Cancel",
      save: "Save",
      create: "Create",
      saving: "Saving…",
      serviceEndpointInvalid: "Service endpoint is invalid.",
      missingNameTitle: "Missing name",
      missingNameBody: "Please enter a key name.",
      missingGroupTitle: "Missing group",
      missingGroupBody: "Select a group for this key.",
      updateFailed: "Update failed",
      createFailed: "Create key failed",
      cannotApplyKey: "Cannot apply key",
      movedTitle: "Moved to the matching tab",
      movedKeyMessage: (keyName: string, cliLabel: string, cloudName: string) =>
        `Key "${keyName}" belongs to ${cliLabel}. ${cloudName} switched tabs for you so you can apply it there.`,
      globalUpdatedTitle: "Global CLI default updated",
      globalKeyUpdatedMessage: (
        cloudName: string,
        keyName: string,
        cliLabel: string,
        configTarget: string,
      ) =>
        `${cloudName} key "${keyName}" now configures ${cliLabel} only. Updated ${configTarget}.`,
      switchFailed: "Switch failed",
      deleteFailed: "Delete failed",
    },
    paseoCloudRouting: {
      title: "Routing groups",
      hint: "Choose a Cloud group to start using a route. This Settings page applies a group as a global CLI default; the new-agent model selector can set a group only for the current workspace.",
      currentTab: (cliLabel: string, platform: string) =>
        `Current tab: ${cliLabel} using ${platform} groups only.`,
      retry: "Retry",
      loadingGroups: "Loading groups…",
      noGroupsTitle: "No routing groups available",
      noGroupsBody: (platform: string, cliLabel: string, cloudName: string) =>
        `This account does not currently expose any ${platform} routing groups for ${cliLabel}. Add a compatible group in ${cloudName}, or use BYOK for this CLI.`,
      viewScopeRoutes: (cliLabel: string) => `View ${cliLabel} routes`,
      recommended: "Recommended",
      latency: "Latency",
      runtimeUnknown: "Runtime status has not been observed yet for this group.",
      activeRouteVia: (key: string) => `Active route via ${key}`,
      reusableKeyCount: (count: number) =>
        `${count} key${count === 1 ? "" : "s"} available for reuse`,
      noExistingKey: "No existing key yet. Applying this group will create one automatically.",
      recommendedInsight: (cliLabel: string) =>
        `Best available option right now for ${cliLabel} based on health, latency, and price.`,
      downWarning:
        "Recent probes look unhealthy. Prefer another group unless you specifically need this route.",
      advancedAction: "Advanced action: writes",
      applying: "Applying…",
      activeCta: (cliLabel: string) => `Active · ${cliLabel}`,
      setGlobalDefault: "Set as global CLI default",
      serviceEndpointInvalid: "Service endpoint is invalid.",
      cannotUseGroup: "Cannot use group",
      groupUnavailable: "The selected group is no longer available.",
      movedTitle: "Moved to the matching tab",
      movedGroupMessage: (groupName: string, cliLabel: string, cloudName: string) =>
        `Group "${groupName}" belongs to ${cliLabel}. ${cloudName} switched tabs for you so you can apply it there.`,
      defaultKeyName: (groupName: string) => `${groupName} Key`,
      globalUpdatedTitle: "Global CLI default updated",
      globalGroupUpdatedMessage: (groupName: string, cliLabel: string, configTarget: string) =>
        `Group "${groupName}" now configures ${cliLabel} only. Updated ${configTarget}. New workspace sessions can also use group routing from the model selector without changing this global default.`,
      switchFailed: "Switch failed",
    },
    paseoCloudUsage: {
      title: "Usage",
      datePresets: {
        today: "Today",
        "7d": "7 days",
        "30d": "30 days",
      },
      requestTypes: {
        ws: "WS",
        stream: "Stream",
        sync: "Sync",
        unknown: "Unknown",
      },
      billingModes: {
        perRequest: "Per request",
        image: "Image",
        token: "Token",
      },
      unknownKey: "Unknown key",
      keyNumber: (id: number) => `Key #${id}`,
      allApiKeys: "All API keys",
      unknownModel: "Unknown model",
      tokens: "Tokens",
      cache: "Cache",
      cost: "Cost",
      latency: "Latency",
      input: "In",
      output: "Out",
      write: "W",
      read: "R",
      standard: "Standard",
      first: "First",
      noInboundEndpoint: "No inbound endpoint recorded",
      signInHint: (cloudName: string) => `Sign in to ${cloudName} to view usage.`,
      rangeTitle: "Usage range",
      rangeText: (start: string, end: string) => `${start} to ${end}`,
      refresh: "Refresh",
      requests: "Requests",
      recordsInRange: (count: number) => `${count} records in range`,
      actualCost: "Actual cost",
      avgDuration: "Avg duration",
      acrossMatchedRequests: "Across matched requests",
      recordsTitle: "Usage records",
      pageOf: (page: number, pages: number) => `Page ${page} of ${pages}`,
      prev: "Prev",
      next: "Next",
      loading: "Loading usage...",
      noRecords: "No usage records found for this range.",
    },
    paseoCloudModelStatus: {
      title: "Model Status",
      signInHint: (cloudName: string) => `Sign in to ${cloudName} to view model runtime status.`,
      runtimeHealth: "Runtime health",
      refreshInterval: "Refreshes every 30 seconds while this section is open.",
      refresh: "Refresh",
      groups: "Groups",
      loadingGroups: "Loading group statuses...",
      noGroups: (cloudName: string) => `No monitored groups returned by ${cloudName}.`,
      statusLabels: {
        up: "Healthy",
        degraded: "Degraded",
        down: "Unavailable",
        unknown: "Unknown",
      },
      observed: "observed",
      latency: "Latency",
      availability: "Availability",
      average: "avg",
      probes: "probes",
      detailHint: "History, recent probe records, and status events.",
      loadingDetails: "Loading status details...",
      availabilityHistory: "Availability history",
      noHistory: "No history buckets yet.",
      recentProbes: "Recent probes",
      noRecords: "No probe records yet.",
      statusEvents: "Status events",
      noEvents: "No status events yet.",
      statusFallback: "status",
      unknownStatus: "unknown",
      http: "HTTP",
    },
    paseoCloudReferral: {
      title: "Referral",
      loading: "Loading referral info...",
      referralCode: "Referral Code",
      copy: "Copy",
      copied: "Copied",
      referralLink: "Referral Link",
      share: "Share",
      total: "Total",
      rewarded: "Rewarded",
      pending: "Pending",
      earned: "Earned",
      balance: "balance",
      subscriptionDays: (days: number) => `${days}d subscription`,
      referrerReward: (balance: string, subscription: string | null) =>
        `Referrer: ${balance} balance${subscription ? ` + ${subscription}` : ""}`,
      refereeReward: (balance: string, subscription: string | null) =>
        `Referee: ${balance} balance${subscription ? ` + ${subscription}` : ""}`,
      historyTitle: "Referral history",
      statusLabel: (status: string) => status,
    },
    settings: {
      title: "Settings",
      loadingSettings: "Loading settings...",
      back: "Back",
      local: "Local",
      addHost: "Add host",
      sections: {
        general: "General",
        paseoCloud: "Cloud service",
        managedProvider: "Provider",
        shortcuts: "Shortcuts",
        integrations: "Integrations",
        permissions: "Permissions",
        diagnostics: "Diagnostics",
        about: "About",
      },
      general: "General",
      theme: "Theme",
      language: "Language",
      languageHint: "Used for the client UI, cloud service, payments, and the model catalog",
      defaultSend: "Default send",
      defaultSendHint: "What happens when you press Enter while the agent is running",
      interrupt: "Interrupt",
      queue: "Queue",
      themes: {
        light: "Light",
        dark: "Dark",
        zinc: "Zinc",
        midnight: "Midnight",
        claude: "Claude",
        ghostty: "Ghostty",
        auto: "System",
      },
      languages: {
        auto: "System",
        zh: "中文",
        en: "English",
      },
      diagnostics: "Diagnostics",
      testAudio: "Test audio",
      playing: "Playing...",
      playTest: "Play test",
      playbackFailed: (message: string) => `Playback failed: ${message}`,
      about: "About",
      version: "Version",
      releaseChannel: "Release channel",
      releaseChannelHint: "Switch to Beta to get updates sooner and help shape them",
      stable: "Stable",
      beta: "Beta",
      appUpdates: "App updates",
      readyToInstall: (version: string) => `Ready to install: ${version}`,
      check: "Check",
      checking: "Checking...",
      update: "Update",
      updateTo: (version: string) => `Update to ${version}`,
      installing: "Installing...",
      installDesktopUpdate: "Install desktop update",
      installDesktopUpdateMessage: (appName: string) => `This updates ${appName} on this computer`,
      installUpdate: "Install update",
      cancel: "Cancel",
      error: "Error",
      unableOpenUpdateConfirmation: "Unable to open the update confirmation dialog.",
      managedProviderTitle: "This device",
      managedProviderBody:
        "Active Claude Code and Codex API routes on this computer, plus saved and custom endpoints. You can point each CLI at a different saved entry.",
      managedProviderCloudHint: (cloudName: string) =>
        `${cloudName} (account, keys, billing) lives in the sidebar under ${cloudName}.`,
      paseoCloudBody:
        "Account sign-in, balance, routing groups, API keys, and the model catalog for the managed service. This is separate from the on-device Claude/Codex routes under Provider.",
    },
  },
} as const;

export function getSub2APIMessages(locale: string | null | undefined = DEFAULT_LOCALE) {
  return sub2apiMessages[normalizeSub2APILocale(locale)];
}
