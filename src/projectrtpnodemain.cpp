
#ifdef NODE_MODULE

#include <node_api.h>
#include <string>
#include <vector>

#include <atomic>
#include <thread>

#include <sys/time.h>
#include <sys/resource.h>

#include <boost/asio.hpp>
#include <boost/exception/diagnostic_information.hpp>

#include "projectrtpnodemain.h"
#include "projectrtpbuffer.h"
#include "projectrtpchannel.h"
#include "projectrtpsoundfile.h"
#include "projectrtptonegen.h"
#include "projectrtpsrtp.h"

boost::asio::io_context workercontext;
static napi_async_work workhandle;
static bool started = false;
static std::atomic_bool warningissued( false );
int32_t startport = 10000;
int32_t endport = 20000;

/* our work queue requires some work to not exit */
static ourhighrestimer periodictimer( workercontext );

std::atomic_bool running;

static void ontimer( const boost::system::error_code& e ) {
  if ( e == boost::asio::error::operation_aborted ) return;
  if( !running ) return;

  periodictimer.expires_at( periodictimer.expiry() + std::chrono::seconds( 1 ) );
  periodictimer.async_wait( &ontimer );
}

static void workerbee( void ) {

  if( 0 != setpriority( PRIO_PROCESS, 0, -20 ) ) {
    if( !warningissued.exchange( true, std::memory_order_acquire ) ) {
      std::cerr << "Warning: failed to set high priority for worker bees" << std::endl;
    }
  }
  
  while( running ) {
    workercontext.run();
  }
}

static void runwork( napi_env env, void *data ) {

  if( running ) return;
  running = true;
  warningissued = false;

  auto numcpus = std::thread::hardware_concurrency();
  std::vector< std::thread > threads( numcpus );

  if( workercontext.stopped() ) {
    workercontext.restart();
  }

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

  dtlsdestroy();
}

static napi_deferred stoppingdefferedpromise;
static void runworkcomplete( napi_env env, napi_status status, void *data ) {
  napi_value resolution;

  napi_create_string_utf8( env, "Server stopped", NAPI_AUTO_LENGTH, &resolution );
  napi_resolve_deferred( env, stoppingdefferedpromise, resolution );

  started = false;
  napi_delete_async_work( env, workhandle );
}

static napi_value stopserver( napi_env env, napi_callback_info info ) {

  /* If we are not running we may already have an outstanding unresolved promise */
  if( !running ) return NULL;
  napi_value stoppingpromise;

  if( napi_ok != napi_create_promise( env, &stoppingdefferedpromise, &stoppingpromise ) ) {
    return NULL;
  }

  running = false;
  periodictimer.cancel();

  return stoppingpromise;
}

static napi_value startserver( napi_env env, napi_callback_info info ) {

  if( started ) return NULL;
  dtlsinit();

  started = true;
  napi_value nports;
  napi_value nstartport;
  napi_value nendport;
  napi_value argv[ 1 ];
  size_t argc = 1;
  napi_value result;
  bool hasit;
  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;
  if( 1 == argc ) {
    if( napi_ok == napi_has_named_property( env, argv[ 0 ], "ports", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, argv[ 0 ], "ports", &nports ) ) 
    {
    if( napi_ok == napi_has_named_property( env, nports, "start", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, nports, "start", &nstartport ) ) {
        napi_get_value_int32( env, nstartport, &startport );
      }
    if( napi_ok == napi_has_named_property( env, nports, "end", &hasit ) &&
        hasit &&
        napi_ok == napi_get_named_property( env, nports, "end", &nendport ) ) {
        napi_get_value_int32( env, nendport, &endport );
      }

      initrtpchannel( env, result, startport, endport );
    }
  }
  
  napi_value workname;

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

/* channel at the moment - but future proof to include other stats */
static napi_value stats( napi_env env, napi_callback_info info ) {
  napi_value result;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  getchannelstats( env, result );

  auto numcpus = std::thread::hardware_concurrency();

  napi_value wc;
  if( napi_ok != napi_create_double( env, numcpus, &wc ) ) return NULL;
  if( napi_ok != napi_set_named_property( env, result, "workercount", wc ) ) return NULL;

  return result;
}

void initserver( napi_env env, napi_value &result ) {

  napi_value bstopserver, bstartserver, cstats;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, startserver, nullptr, &bstartserver ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "run", bstartserver ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, stopserver, nullptr, &bstopserver ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "shutdown", bstopserver ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, stats, nullptr, &cstats ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "stats", cstats ) ) return;
}

/* Initialize the module */
napi_value initprtp(napi_env env, napi_value exports) {
    std::atomic_bool test;
    if (!test.is_lock_free()) {
        fprintf(stderr, "Warning - performance will be poor as atomic variables are not available\n");
    }

    srand(time(NULL));

    /* Initialize DTLS or other dependencies */
    dtlsinit();

    /* Initialize the exported module object */
    initserver(env, exports);
    initrtpbuffer(env, exports);
    initrtpchannel(env, exports, startport, endport);
    initrtpsoundfile(env, exports);
    initrtpcodecx(env, exports);
    initfilter(env, exports);
    inittonegen(env, exports);
    initsrtp(env, exports);

    /* Generate G.711 conversion data */
    gen711convertdata();

    return exports;
}

/* Register the module */
NAPI_MODULE(NODE_GYP_MODULE_NAME, initprtp)

#endif /* NODE_MODULE */
