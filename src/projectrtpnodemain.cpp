
#ifdef NODE_MODULE

#include <node_api.h>
#include <string>
#include <iostream> // cout
#include <algorithm> // min

#include "projectrtpbuffer.h"


napi_value init( napi_env env, napi_value exports ) {
  napi_value result;

  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  /* Init our modules */
  initrtpbuffer( env, result );

  return result;
}

NAPI_MODULE( NODE_GYP_MODULE_NAME, init )

#endif /* NODE_MODULE */
