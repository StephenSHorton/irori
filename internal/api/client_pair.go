package api

import (
	"context"
	"errors"
	"net/http"

	"github.com/StephenSHorton/irori/internal/pair"
)

// PairJoin enters a code on this device (POST /v1/pair/join). server is the
// home server's address, "" at home.
func (c *Client) PairJoin(ctx context.Context, code, server string) (pair.Status, error) {
	var st pair.Status
	err := c.impl.do(ctx, http.MethodPost, pathPairJoin, nil, Identity{}, nil,
		map[string]string{"code": code, "server": server}, &st)
	return st, err
}

// PairStatus is the joining side's pairing (GET /v1/pair/status).
func (c *Client) PairStatus(ctx context.Context) (pair.Status, error) {
	var st pair.Status
	err := c.impl.get(ctx, pathPairStatus, nil, true, &st)
	return st, err
}

// PairKey types the other device's key (POST /v1/pair/key). A wrong key is
// an error that IsWrongKey reports.
func (c *Client) PairKey(ctx context.Context, key string) (pair.Status, error) {
	var st pair.Status
	err := c.impl.do(ctx, http.MethodPost, pathPairKey, nil, Identity{}, nil,
		map[string]string{"key": key}, &st)
	return st, err
}

// PairCancel closes this device's join, if one is open (POST
// /v1/pair/cancel). It never closes an Add window.
func (c *Client) PairCancel(ctx context.Context) (pair.Status, error) {
	var st pair.Status
	err := c.impl.do(ctx, http.MethodPost, pathPairCancel, nil, Identity{}, nil, nil, &st)
	return st, err
}

// IsWrongKey reports whether err is the server refusing a key (422).
func IsWrongKey(err error) bool {
	var se *StatusError
	return errors.As(err, &se) && se.Code == http.StatusUnprocessableEntity
}

// IsNoRoute reports whether err is the server not knowing the route: a
// Core older than the pairing routes.
func IsNoRoute(err error) bool {
	var se *StatusError
	return errors.As(err, &se) && (se.Code == http.StatusNotFound || se.Code == http.StatusMethodNotAllowed)
}
