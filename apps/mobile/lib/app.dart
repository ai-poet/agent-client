import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_slidable/flutter_slidable.dart';
import 'package:fluttertoast/fluttertoast.dart';
import 'features/conversations/tasks_page.dart';
import 'features/notifications/delivery.dart';
import 'features/resources/hosts_page.dart';
import 'features/resources/projects_page.dart';
import 'features/settings/settings_page.dart';
import 'ui/app_background.dart';
import 'ui/kit.dart';
import 'ui/theme.dart';
import 'ui/toast.dart';
import 'runtime/session.dart';
import 'runtime/preferences.dart';
import 'l10n/strings.dart';
import 'l10n/language.dart';
import 'package:shared_preferences/shared_preferences.dart';

class SailryApp extends StatefulWidget {
  const SailryApp({super.key, this.session, this.preferences});
  final AppSession? session;
  final SharedPreferencesAsync? preferences;
  @override
  State<SailryApp> createState() => _SailryAppState();
}

class _SailryAppState extends State<SailryApp> {
  AppSession? _session;
  final _navigator = GlobalKey<NavigatorState>();
  late final _preferences = AppPreferences(
    preferences: widget.preferences,
    persist: _persist,
    onError: _preferenceError,
  );
  bool get _persist => widget.session == null || widget.preferences != null;
  @override
  void initState() {
    super.initState();
    _session = widget.session ?? AppSession();
    if (widget.session == null) unawaited(_session!.start());
    _preferences.addListener(_refreshPreferences);
    if (_persist) unawaited(_preferences.load());
  }

  void _refreshPreferences() {
    if (mounted) setState(() {});
  }

  void _preferenceError() {
    if (!mounted) return;
    final context = _navigator.currentContext;
    if (context != null) showToast(context, context.tr('preferencesFailed'));
  }

  @override
  void dispose() {
    _preferences.removeListener(_refreshPreferences);
    _preferences.dispose();
    FToast().removeQueuedCustomToasts();
    if (widget.session == null) _session?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final app = MaterialApp(
      title: 'CheapRouter Harness',
      navigatorKey: _navigator,
      debugShowCheckedModeBanner: false,
      theme: SailryTheme.of(Brightness.light, _preferences.brand),
      darkTheme: SailryTheme.of(Brightness.dark, _preferences.brand),
      themeMode: _preferences.mode,
      locale: _preferences.language.locale,
      localeListResolutionCallback: AppLanguage.resolve,
      supportedLocales: AppLocalizations.supportedLocales,
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      builder: (context, navigator) => FToastBuilder()(
        context,
        // The toast overlay retains its initial child. Observe the session
        // inside that subtree so pairing updates the mounted application frame.
        ListenableBuilder(
          listenable: _session!,
          child: navigator,
          builder: (context, child) {
            _session?.background.localize(context.tr);
            _session?.localize(context.tr);
            return ColoredBox(
              color: Theme.of(context).colorScheme.surface,
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 520),
                  child: AppBackground(child: child!),
                ),
              ),
            );
          },
        ),
      ),
      home: _home(),
    );
    return _session == null
        ? app
        : SessionScope(session: _session!, child: app);
  }

  Widget _home() {
    Widget home = _Shell(
      language: _preferences.language,
      onLanguageChanged: _preferences.setLanguage,
      themeMode: _preferences.mode,
      onThemeChanged: _preferences.setTheme,
      brand: _preferences.brand,
      onBrandChanged: _preferences.setBrand,
      notifications: _preferences.notifications,
      onNotificationsChanged: _preferences.setNotifications,
    );
    if (_session != null) {
      home = NotificationDelivery(
        session: _session!,
        enabled:
            _preferences.notifications &&
            (!_persist ||
                _preferences.loaded ||
                _preferences.notificationsEdited),
        child: home,
      );
    }
    return home;
  }
}

class _Shell extends StatefulWidget {
  const _Shell({
    required this.language,
    required this.onLanguageChanged,
    required this.themeMode,
    required this.onThemeChanged,
    required this.brand,
    required this.onBrandChanged,
    required this.notifications,
    required this.onNotificationsChanged,
  });
  final AppLanguage language;
  final ValueChanged<AppLanguage> onLanguageChanged;
  final ThemeMode themeMode;
  final ValueChanged<ThemeMode> onThemeChanged;
  final Brand brand;
  final ValueChanged<Brand> onBrandChanged;
  final bool notifications;
  final ValueChanged<bool> onNotificationsChanged;
  @override
  State<_Shell> createState() => _ShellState();
}

class _ShellState extends State<_Shell> {
  int _index = 1;
  bool _resolvedInitialTab = false;
  bool _userSelectedTab = false;
  int _taskRoute = 0;
  String _taskFilter = 'all';
  bool _sessionsOnly = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final session = AppSession.maybeOf(context);
    if (!_resolvedInitialTab && session != null && session.ready) {
      _resolvedInitialTab = true;
      if (!_userSelectedTab && session.hosts.isNotEmpty) {
        _index = 0;
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final session = AppSession.maybeOf(context);
    return Scaffold(
      extendBody: true,
      backgroundColor: Colors.transparent,
      body: SlidableAutoCloseBehavior(
        child: session != null && !session.ready && session.error != null
            ? PageFrame(
                title: context.tr('brand'),
                failure: FailureState(
                  message: context.tr('startupFailed'),
                  onRetry: session.start,
                ),
                child: const SizedBox.shrink(),
              )
            : IndexedStack(
                index: _index,
                children:
                    <Widget>[
                          TasksPage(
                            key: ValueKey(_taskRoute),
                            initialFilter: _taskFilter,
                            sessionsOnly: _sessionsOnly,
                          ),
                          HostsPage(
                            onBrowse: (kind) => setState(() {
                              _userSelectedTab = true;
                              _taskRoute++;
                              _taskFilter = kind == 'terminals'
                                  ? 'terminal'
                                  : 'all';
                              _sessionsOnly = kind == 'sessions';
                              _index = 0;
                            }),
                          ),
                          const ProjectsPage(),
                          SettingsPage(
                            language: widget.language,
                            onLanguageChanged: widget.onLanguageChanged,
                            themeMode: widget.themeMode,
                            onThemeChanged: widget.onThemeChanged,
                            brand: widget.brand,
                            onBrandChanged: widget.onBrandChanged,
                            notifications: widget.notifications,
                            onNotificationsChanged:
                                widget.onNotificationsChanged,
                          ),
                        ].indexed
                        .map(
                          (entry) => TickerMode(
                            enabled: entry.$1 == _index,
                            child: entry.$2,
                          ),
                        )
                        .toList(),
              ),
      ),
      bottomNavigationBar: FloatingNavigation(
        selected: _index,
        onSelected: (index) => setState(() {
          _userSelectedTab = true;
          if (index == 0 && (_taskFilter != 'all' || _sessionsOnly)) {
            _taskRoute++;
            _taskFilter = 'all';
            _sessionsOnly = false;
          }
          _index = index;
        }),
      ),
    );
  }
}
