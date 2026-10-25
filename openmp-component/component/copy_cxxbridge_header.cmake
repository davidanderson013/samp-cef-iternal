file(GLOB CXXBRIDGE_HEADERS
	"${RUST_LIB_DIR}/build/cef-openmp-*/out/cxxbridge/include/cef-openmp/src/lib.rs.h"
)

list(LENGTH CXXBRIDGE_HEADERS CXXBRIDGE_HEADER_COUNT)
if(CXXBRIDGE_HEADER_COUNT EQUAL 0)
	message(FATAL_ERROR "Could not find generated cxxbridge header under ${RUST_LIB_DIR}/build")
endif()

list(SORT CXXBRIDGE_HEADERS)
list(GET CXXBRIDGE_HEADERS -1 CXXBRIDGE_HEADER)

file(MAKE_DIRECTORY "${GENERATED_INCLUDE_DIR}/cef-openmp/src")
file(COPY_FILE
	"${CXXBRIDGE_HEADER}"
	"${GENERATED_INCLUDE_DIR}/cef-openmp/src/lib.rs.h"
	ONLY_IF_DIFFERENT
)
