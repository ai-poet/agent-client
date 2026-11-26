package live

import (
	"context"
	"errors"
	"log/slog"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
)

type cheapGatewayRead struct {
	user                        core.User
	gateway                     core.Gateway
	groups                      []core.Group
	statuses                    []core.GroupStatus
	accountErr, groupsErr       error
	gatewayErr, statusErr       error
	authErr                     error // definite rejection, after any origin recovery
	statusAttempted, sameOrigin bool
	autoBindAttempted           bool
	autoBindFailures            map[string]string
	recoveringOrigin            bool
}

// Origin recovery needs a fresh catalog and keys as well as an account check:
// the previous Node route may point at a third origin, not the login origin.
func readCheapGateway(ctx context.Context, service *core.Service, pendingErr error, autoBind bool) cheapGatewayRead {
	login := service.Endpoint()
	read := func(recovering bool) cheapGatewayRead {
		out := cheapGatewayRead{
			gatewayErr: pendingErr, recoveringOrigin: recovering,
			sameOrigin: service.GatewayOriginConfig().ResolveAPIOrigin(login) == login,
		}
		stop := func(err error) bool {
			if !cheapAuthRejected(err) {
				return false
			}
			var auth *core.AuthError
			if errors.As(err, &auth) && auth.RejectedOrigin == login && login != "" && !recovering {
				// An individual API can refuse a renewed bearer without
				// invalidating the login. Keep optional reads optional.
				return false
			}
			out.authErr = err
			return true
		}
		out.user, out.accountErr = service.Account(ctx)
		if stop(out.accountErr) || ctx.Err() != nil {
			return out
		}
		out.groups, out.groupsErr = service.AvailableGroups(ctx)
		if stop(out.groupsErr) {
			return out
		}
		if out.groupsErr != nil && len(service.LaneBindings()) > 0 && out.gatewayErr == nil {
			out.gatewayErr = out.groupsErr
		}
		if out.gatewayErr == nil && ctx.Err() == nil {
			out.gateway, out.gatewayErr = service.GatewayWithGroups(ctx, out.groups)
			if stop(out.gatewayErr) {
				return out
			}
			if autoBind && out.gatewayErr == nil && out.groupsErr == nil {
				if stop(out.autoBind(ctx, service)) {
					return out
				}
			}
		}
		if ctx.Err() == nil {
			out.statusAttempted = true
			out.statuses, out.statusErr = service.GroupStatuses(ctx)
			stop(out.statusErr)
		}
		return out
	}

	out := read(false)
	var auth *core.AuthError
	if ctx.Err() == nil && errors.As(out.authErr, &auth) && auth.RejectedOrigin != "" && auth.RejectedOrigin != login {
		if err := service.RollBackRejectedOrigin(auth.RejectedOrigin); err != nil {
			out.recoveringOrigin = true
			out.authErr, out.gatewayErr = nil, err
			return out
		}
		out = read(true)
		// A concurrent routing change cannot confirm a login rejection.
		if errors.As(out.authErr, &auth) && auth.RejectedOrigin != "" && auth.RejectedOrigin != login {
			out.gatewayErr, out.authErr = out.authErr, nil
		}
	}
	return out
}

func (out *cheapGatewayRead) autoBind(ctx context.Context, service *core.Service) error {
	out.autoBindAttempted = true
	gateway := out.gateway
	updated, result, err := service.AutoBindUnboundLanes(ctx, out.groups,
		&core.ModelCatalog{BaseURL: gateway.BaseURL, Items: gateway.Catalog}, nil)
	var authErr error
	if len(result.Failures) > 0 {
		out.autoBindFailures = make(map[string]string, len(result.Failures))
		for _, failure := range result.Failures {
			out.autoBindFailures[failure.Lane] = cheapAuthError(failure.Err)
			if cheapAuthRejected(failure.Err) {
				authErr = failure.Err
			}
		}
	}
	if err == nil && len(result.Bound) > 0 {
		_, err = providers.CheapRouterProviders(updated)
		if err == nil {
			out.gateway = updated
		}
	}
	if err != nil {
		slog.Warn("cheaprouter auto-bind gateway refresh failed", "error", err.Error())
		for lane := range result.Bound {
			_ = service.RestoreLaneBinding(ctx, lane, nil, false)
		}
		if cheapAuthRejected(err) {
			return err
		}
	}
	return authErr
}
