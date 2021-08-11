
#ifdef NODE_MODULE

#include <node_api.h>
#include <string>
#include <vector>

#include <atomic>
#include <thread>

#include <boost/asio.hpp>

#include "projectrtpnodemain.h"
#include "projectrtpbuffer.h"
#include "projectrtpchannel.h"

std::string mediachroot;
boost::asio::io_context workercontext;
/* our work queue requires some work to not exit */
static ourhighrestimer periodictimer( workercontext );

std::atomic_bool running;

static void ontimer( const boost::system::error_code& e ) {
  if ( e != boost::asio::error::operation_aborted ) {
    if( running ) {
      periodictimer.expires_at( periodictimer.expiry() + std::chrono::seconds( 1 ) );
      periodictimer.async_wait( &ontimer );
    }
  }
}

static void workerbee( void ) {
  while( running ) {
    workercontext.run();
  }
}

static void runwork( napi_env env, void *data ) {

  running = true;

  auto numcpus = std::thread::hardware_concurrency();
  std::vector< std::thread > threads( numcpus );

  boost::asio::post( workercontext, [](){
    periodictimer.expires_after( std::chrono::seconds( 1 ) );
    periodictimer.async_wait( &ontimer );
  } );

  for ( unsigned i = 0; i < numcpus; i++ ) {
    threads[ i ] = std::thread( workerbee );
  }

  for ( auto& t : threads ) {
    t.join();
  }
}

static napi_deferred stoppingdefferedpromise;
static void runworkcomplete( napi_env env, napi_status status, void *data ) {
  napi_value resolution;

  napi_create_string_utf8( env, "Server stopped", NAPI_AUTO_LENGTH, &resolution );
  napi_resolve_deferred( env, stoppingdefferedpromise, resolution );
}

static napi_value stopserver( napi_env env, napi_callback_info info ) {

  napi_value stoppingpromise;

  if( napi_ok != napi_create_promise( env, &stoppingdefferedpromise, &stoppingpromise ) ) {
    return NULL;
  }

  running = false;
  periodictimer.cancel();

  return stoppingpromise;
}

static napi_value startserver( napi_env env, napi_callback_info info ) {

  napi_value workname;
  napi_async_work workhandle;

  if( napi_ok != napi_create_string_utf8( env, "projectrtp", NAPI_AUTO_LENGTH, &workname ) ) {
    return NULL;
  }

  if( napi_ok != napi_create_async_work( env,
                                         NULL,
                                         workname,
                                         runwork,
                                         runworkcomplete,
                                         NULL, /*void* data,*/
                                         &workhandle ) ) {
    return NULL;
  }

  if( napi_ok != napi_queue_async_work( env, workhandle ) ) {
    return NULL;
  }

  return NULL;
}


void initserver( napi_env env, napi_value &result ) {

  napi_value bstopserver, bstartserver;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, startserver, nullptr, &bstartserver ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "run", bstartserver ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, stopserver, nullptr, &bstopserver ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "shutdown", bstopserver ) ) return;
}

napi_value init( napi_env env, napi_value exports ) {
  napi_value result;

  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  /* Init our modules */
  initserver( env, result );
  initrtpbuffer( env, result );
  initrtpchannel( env, result );

  return result;
}

NAPI_MODULE( NODE_GYP_MODULE_NAME, init )

#endif /* NODE_MODULE */